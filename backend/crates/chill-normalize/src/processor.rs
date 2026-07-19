use std::time::Duration;

use chill_control_plane::{ControlPlaneError, Store};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction, types::Json};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::time::MissedTickBehavior;
use tracing::error;

use crate::validator::validate_and_filter;
use crate::{Decoder, InboxItem, Record};

/// Durable worker configuration.
#[derive(Clone, Debug)]
pub struct ProcessorConfiguration {
    /// Stable lease owner identifier.
    pub worker_id: String,
    /// Lease lifetime before another worker may recover an item.
    pub lease_duration: Duration,
    /// Idle polling interval.
    pub poll_interval: Duration,
    /// Delivery attempts before dead-lettering.
    pub maximum_attempts: i32,
    /// Maximum items processed in one pass.
    pub batch_size: usize,
}

impl ProcessorConfiguration {
    /// Returns production defaults for one worker identity.
    #[must_use]
    pub fn production(worker_id: impl Into<String>) -> Self {
        Self {
            worker_id: worker_id.into(),
            lease_duration: Duration::from_secs(30),
            poll_interval: Duration::from_millis(250),
            maximum_attempts: 8,
            batch_size: 64,
        }
    }

    fn validate(&self) -> Result<(), ProcessorError> {
        if self.worker_id.is_empty()
            || self.worker_id.len() > 128
            || self.lease_duration.is_zero()
            || self.poll_interval.is_zero()
            || self.maximum_attempts < 1
            || !(1..=10_000).contains(&self.batch_size)
        {
            return Err(ProcessorError::InvalidConfiguration);
        }
        Ok(())
    }
}

/// Normalization worker failure.
#[derive(Debug, Error)]
pub enum ProcessorError {
    /// Worker limits or identity are invalid.
    #[error("normalizer processor configuration is invalid")]
    InvalidConfiguration,
    /// Tenant context or database work failed.
    #[error("normalization database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// The control-plane tenant boundary rejected an operation.
    #[error("normalization control-plane operation failed: {0}")]
    Control(#[from] ControlPlaneError),
    /// A leased item changed ownership before transition.
    #[error("normalizer lost its inbox lease")]
    LostLease,
    /// Canonical content failed a terminal integrity or policy check.
    #[error("normalization integrity failure: {0}")]
    Integrity(String),
}

/// Tenant-safe durable inbox normalization worker.
#[derive(Clone)]
pub struct Processor {
    pool: PgPool,
    control: Store,
    decoder: Decoder,
    configuration: ProcessorConfiguration,
}

impl Processor {
    /// Creates a worker after validating its bounded configuration.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid lease, polling, attempt, batch, or identity values.
    pub fn new(
        pool: PgPool,
        control: Store,
        decoder: Decoder,
        configuration: ProcessorConfiguration,
    ) -> Result<Self, ProcessorError> {
        configuration.validate()?;
        Ok(Self {
            pool,
            control,
            decoder,
            configuration,
        })
    }

    /// Runs normalization passes until the task is cancelled.
    pub async fn run(&self) {
        let mut ticker = tokio::time::interval(self.configuration.poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            match self.process_available().await {
                Ok(0) => {
                    ticker.tick().await;
                }
                Ok(_) => {}
                Err(error) => {
                    error!(%error, "normalization pass failed");
                    ticker.tick().await;
                }
            }
        }
    }

    /// Processes up to one configured batch.
    ///
    /// # Errors
    ///
    /// Returns a database or tenant-boundary failure. Item decode failures are dead-lettered.
    pub async fn process_available(&self) -> Result<usize, ProcessorError> {
        let organizations = sqlx::query_scalar::<_, String>(
            "SELECT organization_id::text FROM control.list_active_organization_ids() AS organization_id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut processed = 0;
        while processed < self.configuration.batch_size {
            let mut found = false;
            for organization in &organizations {
                let Some(item) = self.lease(organization).await? else {
                    continue;
                };
                found = true;
                self.process_item(&item).await?;
                processed += 1;
                break;
            }
            if !found {
                break;
            }
        }
        Ok(processed)
    }

    /// Processes one configured batch for a specific organization.
    ///
    /// This is useful for deterministic tenant-scoped maintenance and testing;
    /// the normal run loop uses round-robin organization discovery.
    ///
    /// # Errors
    ///
    /// Returns a database or tenant-boundary failure. Decode failures are dead-lettered.
    pub async fn process_organization(&self, organization: &str) -> Result<usize, ProcessorError> {
        let mut processed = 0;
        while processed < self.configuration.batch_size {
            let Some(item) = self.lease(organization).await? else {
                break;
            };
            self.process_item(&item).await?;
            processed += 1;
        }
        Ok(processed)
    }

    async fn process_item(&self, item: &InboxItem) -> Result<(), ProcessorError> {
        match self.decoder.decode(item) {
            Ok(records) => {
                if let Err(error) = self.complete(item, &records).await {
                    if let ProcessorError::Integrity(message) = &error {
                        self.transition_failure(
                            item,
                            "dead_letter",
                            "normalization.integrity",
                            message,
                            Duration::ZERO,
                        )
                        .await?;
                    } else {
                        self.retry(item, "normalization.persist", &error.to_string())
                            .await?;
                    }
                }
            }
            Err(error) => {
                self.transition_failure(
                    item,
                    "dead_letter",
                    "normalization.decode",
                    &error.to_string(),
                    Duration::ZERO,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn lease(&self, organization: &str) -> Result<Option<InboxItem>, ProcessorError> {
        let mut transaction = self.control.begin_tenant(organization).await?;
        let lease_seconds =
            i32::try_from(self.configuration.lease_duration.as_secs()).unwrap_or(i32::MAX);
        let row = sqlx::query_as::<_, InboxRow>(
            r"
            WITH candidate AS (
                SELECT id FROM ingest.inbox
                WHERE ((status='pending' AND available_at <= clock_timestamp())
                    OR (status='leased' AND lease_expires_at <= clock_timestamp()))
                ORDER BY available_at, id FOR UPDATE SKIP LOCKED LIMIT 1
            )
            UPDATE ingest.inbox AS inbox
            SET status='leased', lease_owner=$1,
                lease_expires_at=clock_timestamp()+make_interval(secs => $2),
                attempt_count=inbox.attempt_count+1,
                last_error_code=NULL,last_error_message=NULL
            FROM candidate WHERE inbox.id=candidate.id
            RETURNING inbox.id,inbox.organization_id::text,inbox.project_id::text,
                inbox.environment_id::text,inbox.data_source_id::text,inbox.signal_kind,
                inbox.payload_format,inbox.payload,inbox.metadata,inbox.server_received_at,
                inbox.attempt_count
            ",
        )
        .bind(&self.configuration.worker_id)
        .bind(lease_seconds)
        .fetch_optional(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(row.map(InboxRow::into_item))
    }

    async fn complete(&self, item: &InboxItem, records: &[Record]) -> Result<(), ProcessorError> {
        let mut transaction = self.control.begin_tenant(&item.organization_id).await?;
        let records = validate_and_filter(&mut transaction, item, records).await?;
        for record in &records {
            persist_record(&mut transaction, item, record).await?;
        }
        let affected = sqlx::query(
            r"
            UPDATE ingest.inbox SET status='completed',completed_at=clock_timestamp(),
                lease_owner=NULL,lease_expires_at=NULL,last_error_code=NULL,last_error_message=NULL,
                payload=$3,
                metadata=CASE WHEN signal_kind='replay'
                    THEN jsonb_build_object('payload_disposition','canonicalized')
                    ELSE metadata || jsonb_build_object('payload_disposition','canonicalized') END
            WHERE id=$1 AND status='leased' AND lease_owner=$2
            ",
        )
        .bind(item.id)
        .bind(&self.configuration.worker_id)
        .bind(b"canonicalized".as_slice())
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(ProcessorError::LostLease);
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn retry(
        &self,
        item: &InboxItem,
        code: &str,
        message: &str,
    ) -> Result<(), ProcessorError> {
        if item.attempt_count >= self.configuration.maximum_attempts {
            return self
                .transition_failure(item, "dead_letter", code, message, Duration::ZERO)
                .await;
        }
        let exponent = u32::try_from((item.attempt_count - 1).clamp(0, 8)).unwrap_or_default();
        self.transition_failure(
            item,
            "pending",
            code,
            message,
            Duration::from_secs(1_u64 << exponent),
        )
        .await
    }

    async fn transition_failure(
        &self,
        item: &InboxItem,
        status: &str,
        code: &str,
        message: &str,
        delay: Duration,
    ) -> Result<(), ProcessorError> {
        let mut transaction = self.control.begin_tenant(&item.organization_id).await?;
        let delay = i32::try_from(delay.as_secs()).unwrap_or(i32::MAX);
        let message = truncate(message, 512);
        let affected = sqlx::query(
            r"
            UPDATE ingest.inbox SET status=$1,
                available_at=clock_timestamp()+make_interval(secs => $2),
                lease_owner=NULL,lease_expires_at=NULL,completed_at=NULL,
                last_error_code=$3,last_error_message=$4
            WHERE id=$5 AND status='leased' AND lease_owner=$6
            ",
        )
        .bind(status)
        .bind(delay)
        .bind(code)
        .bind(message)
        .bind(item.id)
        .bind(&self.configuration.worker_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(ProcessorError::LostLease);
        }
        transaction.commit().await?;
        Ok(())
    }
}

type InboxRow = (
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    Vec<u8>,
    Json<Value>,
    OffsetDateTime,
    i32,
);

trait IntoInboxItem {
    fn into_item(self) -> InboxItem;
}

impl IntoInboxItem for InboxRow {
    fn into_item(self) -> InboxItem {
        InboxItem {
            id: self.0,
            organization_id: self.1,
            project_id: self.2,
            environment_id: self.3,
            data_source_id: self.4,
            signal_kind: self.5,
            payload_format: self.6,
            payload: self.7,
            metadata: self.8.0,
            server_received_at: self.9,
            attempt_count: self.10,
        }
    }
}

async fn persist_record(
    transaction: &mut Transaction<'_, Postgres>,
    item: &InboxItem,
    record: &Record,
) -> Result<(), ProcessorError> {
    let affected = sqlx::query(
        r"
        INSERT INTO ingest.canonical_envelopes (
            organization_id,project_id,environment_id,data_source_id,inbox_id,ordinal,
            envelope_kind,record_id,record_sha256,installation_id,session_id,replay_id,
            replay_chunk_id,trace_id,span_id,occurred_at_unix_nano,
            source_observed_at_unix_nano,server_received_at_unix_nano,
            effective_occurred_at_unix_nano,monotonic_nano,boot_id,sequence_number,
            clock_skew_nano,timing_class,late_arrival,canonical
        ) VALUES (
            $1::uuid,$2::uuid,$3::uuid,$4::uuid,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,
            $16::numeric,$17::numeric,$18::numeric,$19::numeric,$20::numeric,$21,
            $22::numeric,$23::numeric,$24,$25,$26
        ) ON CONFLICT DO NOTHING
        ",
    )
    .bind(&item.organization_id)
    .bind(&item.project_id)
    .bind(&item.environment_id)
    .bind(&item.data_source_id)
    .bind(item.id)
    .bind(record.ordinal)
    .bind(&record.envelope_kind)
    .bind(&record.record_id)
    .bind(record.digest.as_slice())
    .bind(&record.installation_id)
    .bind(&record.session_id)
    .bind(&record.replay_id)
    .bind(&record.replay_chunk_id)
    .bind(&record.trace_id)
    .bind(&record.span_id)
    .bind(decimal(record.occurred_at_unix_nano))
    .bind(decimal(record.source_observed_at_unix_nano))
    .bind(record.timing.server_received_nano.to_string())
    .bind(record.timing.effective_occurred.to_string())
    .bind(decimal(record.monotonic_nano))
    .bind(&record.boot_id)
    .bind(decimal(record.sequence_number))
    .bind(record.timing.clock_skew_nano.to_string())
    .bind(record.timing.class.as_str())
    .bind(record.timing.late)
    .bind(Json(&record.canonical))
    .execute(&mut **transaction)
    .await?
    .rows_affected();
    if affected == 0 {
        let existing = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT record_sha256 FROM ingest.canonical_envelopes WHERE environment_id=$1::uuid AND envelope_kind=$2 AND record_id=$3",
        )
        .bind(&item.environment_id)
        .bind(&record.envelope_kind)
        .bind(&record.record_id)
        .fetch_one(&mut **transaction)
        .await?;
        if existing.as_slice() != record.digest {
            return Err(ProcessorError::LostLease);
        }
    }
    Ok(())
}

fn decimal(value: Option<u64>) -> Option<String> {
    value.map(|number| number.to_string())
}

fn truncate(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        return value;
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}
