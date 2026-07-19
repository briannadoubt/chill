use std::time::Duration;

use bytes::Bytes;
use chill_control_plane::{ControlPlaneError, Store};
use chill_objects::{ImmutableStore, ObjectError};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use time::{Date, OffsetDateTime};
use tokio::time::MissedTickBehavior;
use tracing::error;

use crate::{
    Batch, LakeError, Published, Row, deterministic_batch_id, encode_manifest, encode_parquet,
};

/// Bounded lake publisher configuration.
#[derive(Clone, Debug)]
pub struct ProcessorConfiguration {
    /// Stable lease owner.
    pub worker_id: String,
    /// Lease lifetime.
    pub lease_duration: Duration,
    /// Idle polling interval.
    pub poll_interval: Duration,
    /// Attempts before dead-lettering.
    pub maximum_attempts: i32,
    /// Maximum source rows per micro-batch.
    pub batch_size: usize,
    /// Approximate uncompressed source-byte bound.
    pub maximum_batch_bytes: i32,
    /// Minimum committed batches required to compact one partition.
    pub compaction_minimum: usize,
    /// Maximum committed batches merged by one compaction.
    pub compaction_maximum: usize,
    /// Maximum rows in one compacted object.
    pub maximum_compacted_rows: usize,
}

impl ProcessorConfiguration {
    /// Returns production micro-batch defaults.
    #[must_use]
    pub fn production(worker_id: impl Into<String>) -> Self {
        Self {
            worker_id: worker_id.into(),
            lease_duration: Duration::from_mins(2),
            poll_interval: Duration::from_secs(1),
            maximum_attempts: 8,
            batch_size: 2_048,
            maximum_batch_bytes: 32 << 20,
            compaction_minimum: 4,
            compaction_maximum: 16,
            maximum_compacted_rows: 32_768,
        }
    }

    fn validate(&self) -> Result<(), ProcessorError> {
        if self.worker_id.is_empty()
            || self.worker_id.len() > 128
            || self.lease_duration.is_zero()
            || self.poll_interval.is_zero()
            || self.maximum_attempts < 1
            || !(1..=100_000).contains(&self.batch_size)
            || self.maximum_batch_bytes < 1 << 20
            || self.compaction_minimum < 2
            || self.compaction_maximum < self.compaction_minimum
            || self.compaction_maximum > 256
            || self.maximum_compacted_rows < self.compaction_maximum
            || self.maximum_compacted_rows > 1_000_000
        {
            return Err(ProcessorError::InvalidConfiguration);
        }
        Ok(())
    }
}

/// Durable lake publication failure.
#[derive(Debug, Error)]
pub enum ProcessorError {
    /// Worker limits or identity are invalid.
    #[error("lake processor configuration is invalid")]
    InvalidConfiguration,
    /// Database operation failed.
    #[error("lake database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// Tenant context failed.
    #[error("lake tenant operation failed: {0}")]
    Control(#[from] ControlPlaneError),
    /// Parquet or manifest encoding failed.
    #[error("lake format operation failed: {0}")]
    Format(#[from] LakeError),
    /// Immutable object publication failed.
    #[error("lake object operation failed: {0}")]
    Object(#[from] ObjectError),
    /// Trusted numeric state was outside its contract.
    #[error("lake numeric state is invalid")]
    InvalidNumeric,
    /// A batch changed ownership before transition.
    #[error("lake writer lost its batch lease")]
    LostLease,
}

/// Tenant-safe canonical-to-Parquet publisher.
#[derive(Clone)]
pub struct Processor {
    pool: PgPool,
    control: Store,
    objects: ImmutableStore,
    configuration: ProcessorConfiguration,
}

impl Processor {
    /// Creates a bounded publisher.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid worker configuration.
    pub fn new(
        pool: PgPool,
        control: Store,
        objects: ImmutableStore,
        configuration: ProcessorConfiguration,
    ) -> Result<Self, ProcessorError> {
        configuration.validate()?;
        Ok(Self {
            pool,
            control,
            objects,
            configuration,
        })
    }

    /// Runs until the task is cancelled.
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
                    error!(%error, "lake publication pass failed");
                    ticker.tick().await;
                }
            }
        }
    }

    /// Publishes at most one recovered, new micro-batch, or compaction.
    ///
    /// # Errors
    ///
    /// Returns tenant, database, encoding, or object-store failures after scheduling retry.
    pub async fn process_available(&self) -> Result<usize, ProcessorError> {
        let organizations = sqlx::query_scalar::<_, String>(
            "SELECT organization_id::text FROM control.list_active_organization_ids() AS organization_id",
        )
        .fetch_all(&self.pool)
        .await?;
        for organization in organizations {
            let Some(batch) = self.lease_or_create(&organization).await? else {
                continue;
            };
            if let Err(cause) = self.publish(&batch).await {
                self.handle_failure(&batch, &cause).await?;
            }
            return Ok(1);
        }
        Ok(0)
    }

    /// Publishes at most one batch for a specific tenant.
    ///
    /// # Errors
    ///
    /// Returns tenant, database, encoding, or object-store failures after scheduling retry.
    pub async fn process_organization(&self, organization: &str) -> Result<usize, ProcessorError> {
        let Some(batch) = self.lease_or_create(organization).await? else {
            return Ok(0);
        };
        if let Err(cause) = self.publish(&batch).await {
            self.handle_failure(&batch, &cause).await?;
        }
        Ok(1)
    }

    async fn lease_or_create(&self, organization: &str) -> Result<Option<Batch>, ProcessorError> {
        let mut transaction = self.control.begin_tenant(organization).await?;
        if let Some(batch) = self.lease_pending(&mut transaction).await? {
            transaction.commit().await?;
            return Ok(Some(batch));
        }
        let batch = match self
            .create_micro_batch(&mut transaction, organization)
            .await?
        {
            Some(batch) => Some(batch),
            None => {
                self.create_compaction_batch(&mut transaction, organization)
                    .await?
            }
        };
        transaction.commit().await?;
        Ok(batch)
    }

    async fn lease_pending(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<Option<Batch>, ProcessorError> {
        let seconds =
            i32::try_from(self.configuration.lease_duration.as_secs()).unwrap_or(i32::MAX);
        let row = sqlx::query_as::<_, BatchRow>(
            r"
            WITH candidate AS (
                SELECT candidate.batch_id FROM lake.export_batches AS candidate
                WHERE ((candidate.status='pending' AND candidate.available_at<=clock_timestamp())
                    OR (candidate.status='leased' AND candidate.lease_expires_at<=clock_timestamp()))
                  AND NOT EXISTS (SELECT 1 FROM lifecycle.deletion_requests AS request
                    WHERE request.organization_id=candidate.organization_id
                      AND request.status IN ('pending','leased')
                      AND (request.kind='tenant' OR (request.project_id=candidate.project_id
                        AND request.environment_id=candidate.environment_id)))
                ORDER BY candidate.available_at,candidate.created_at,candidate.batch_id
                FOR UPDATE SKIP LOCKED LIMIT 1
            )
            UPDATE lake.export_batches AS batch SET status='leased',lease_owner=$1,
                lease_expires_at=clock_timestamp()+make_interval(secs=>$2),
                attempt_count=batch.attempt_count+1,last_error_code=NULL,last_error_message=NULL
            FROM candidate WHERE batch.batch_id=candidate.batch_id
            RETURNING batch.batch_id,batch.organization_id::text,batch.project_id::text,
                batch.environment_id::text,batch.batch_kind,batch.partition_day,
                batch.partition_hour::integer,batch.envelope_kind,batch.row_count,
                batch.min_server_received_at_unix_nano::text AS min_server,
                batch.max_server_received_at_unix_nano::text AS max_server,
                batch.min_effective_occurred_at_unix_nano::text AS min_occurred,
                batch.max_effective_occurred_at_unix_nano::text AS max_occurred,
                batch.attempt_count
            ",
        )
        .bind(&self.configuration.worker_id)
        .bind(seconds)
        .fetch_optional(&mut **transaction)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let mut batch = row.into_batch()?;
        if batch.kind == "compaction" {
            batch.supersedes = sqlx::query_scalar::<_, String>(
                "SELECT source_batch_id FROM lake.compaction_sources WHERE compaction_batch_id=$1 ORDER BY source_batch_id",
            )
            .bind(&batch.id)
            .fetch_all(&mut **transaction)
            .await?;
            if batch.supersedes.len() < 2 {
                return Err(ProcessorError::InvalidNumeric);
            }
        }
        Ok(Some(batch))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "selection and claims form one serializable micro-batch transaction"
    )]
    async fn create_micro_batch(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        organization: &str,
    ) -> Result<Option<Batch>, ProcessorError> {
        let first = sqlx::query_as::<_, CandidateRow>(CANDIDATE_FIRST_SQL)
            .fetch_optional(&mut **transaction)
            .await?;
        let Some(first) = first else { return Ok(None) };
        let server_nano = parse_u64(&first.server_received_nano)?;
        let partition_time = OffsetDateTime::from_unix_timestamp_nanos(i128::from(server_nano))
            .map_err(|_| ProcessorError::InvalidNumeric)?;
        let partition_day = partition_time.date();
        let partition_hour = partition_time.hour();
        let day_start = partition_time
            .replace_minute(0)
            .and_then(|value| value.replace_second(0))
            .and_then(|value| value.replace_nanosecond(0))
            .map_err(|_| ProcessorError::InvalidNumeric)?;
        let start = u64::try_from(day_start.unix_timestamp_nanos())
            .map_err(|_| ProcessorError::InvalidNumeric)?;
        let end = start.saturating_add(3_600_000_000_000);
        let limit = i64::try_from(self.configuration.batch_size).unwrap_or(i64::MAX);
        let candidates = sqlx::query_as::<_, CandidateRow>(CANDIDATE_PARTITION_SQL)
            .bind(&first.project_id)
            .bind(&first.environment_id)
            .bind(&first.envelope_kind)
            .bind(start.to_string())
            .bind(end.to_string())
            .bind(limit)
            .fetch_all(&mut **transaction)
            .await?;
        let mut selected = Vec::new();
        let mut bytes = 0_i32;
        for candidate in candidates {
            if !selected.is_empty()
                && bytes.saturating_add(candidate.estimated_bytes)
                    > self.configuration.maximum_batch_bytes
            {
                break;
            }
            bytes = bytes.saturating_add(candidate.estimated_bytes);
            selected.push(candidate);
        }
        let mut digests = Vec::with_capacity(selected.len());
        let mut min_server = u64::MAX;
        let mut max_server = 0;
        let mut min_occurred = u64::MAX;
        let mut max_occurred = 0;
        for candidate in &selected {
            let server = parse_u64(&candidate.server_received_nano)?;
            let occurred = parse_u64(&candidate.effective_occurred)?;
            min_server = min_server.min(server);
            max_server = max_server.max(server);
            min_occurred = min_occurred.min(occurred);
            max_occurred = max_occurred.max(occurred);
            digests.push(candidate.record_digest.clone());
        }
        let partition_key = format!(
            "{organization}/{}/{}/{partition_day}T{partition_hour:02}/{}",
            first.project_id, first.environment_id, first.envelope_kind
        );
        let batch = Batch {
            id: deterministic_batch_id("micro", &partition_key, &digests, &[]),
            organization_id: organization.to_owned(),
            project_id: first.project_id,
            environment_id: first.environment_id,
            kind: "micro".to_owned(),
            partition_day,
            partition_hour,
            envelope_kind: first.envelope_kind,
            row_count: selected.len(),
            min_server_received_at_unix_nano: min_server,
            max_server_received_at_unix_nano: max_server,
            min_effective_occurred_at_unix_nano: min_occurred,
            max_effective_occurred_at_unix_nano: max_occurred,
            attempt_count: 1,
            supersedes: Vec::new(),
        };
        insert_batch(transaction, &batch, &self.configuration).await?;
        for (ordinal, candidate) in selected.iter().enumerate() {
            let ordinal = i32::try_from(ordinal).map_err(|_| ProcessorError::InvalidNumeric)?;
            sqlx::query(
                r"INSERT INTO lake.batch_records (batch_id,organization_id,project_id,
                    environment_id,canonical_envelope_id,row_ordinal) VALUES ($1,$2::uuid,$3::uuid,$4::uuid,$5,$6)",
            )
            .bind(&batch.id).bind(&batch.organization_id).bind(&batch.project_id)
            .bind(&batch.environment_id).bind(candidate.id).bind(ordinal)
            .execute(&mut **transaction).await?;
            sqlx::query(
                r"INSERT INTO lake.source_claims (canonical_envelope_id,batch_id,organization_id,
                    project_id,environment_id) VALUES ($1,$2,$3::uuid,$4::uuid,$5::uuid)",
            )
            .bind(candidate.id)
            .bind(&batch.id)
            .bind(&batch.organization_id)
            .bind(&batch.project_id)
            .bind(&batch.environment_id)
            .execute(&mut **transaction)
            .await?;
        }
        Ok(Some(batch))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "selection, source locking, and membership copy are one atomic compaction claim"
    )]
    async fn create_compaction_batch(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        organization: &str,
    ) -> Result<Option<Batch>, ProcessorError> {
        let minimum = i64::try_from(self.configuration.compaction_minimum)
            .map_err(|_| ProcessorError::InvalidNumeric)?;
        let maximum_rows = i32::try_from(self.configuration.maximum_compacted_rows)
            .map_err(|_| ProcessorError::InvalidNumeric)?;
        let partition = sqlx::query_as::<_, CompactionPartition>(COMPACTION_PARTITION_SQL)
            .bind(minimum)
            .bind(maximum_rows)
            .fetch_optional(&mut **transaction)
            .await?;
        let Some(partition) = partition else {
            return Ok(None);
        };
        let maximum = i64::try_from(self.configuration.compaction_maximum)
            .map_err(|_| ProcessorError::InvalidNumeric)?;
        let candidates = sqlx::query_as::<_, CompactionSource>(COMPACTION_SOURCES_SQL)
            .bind(&partition.project_id)
            .bind(&partition.environment_id)
            .bind(partition.partition_day)
            .bind(partition.partition_hour)
            .bind(&partition.envelope_kind)
            .bind(maximum)
            .bind(maximum_rows)
            .fetch_all(&mut **transaction)
            .await?;
        let mut source_ids = Vec::new();
        let mut row_count = 0_usize;
        let mut min_server = u64::MAX;
        let mut max_server = 0_u64;
        let mut min_occurred = u64::MAX;
        let mut max_occurred = 0_u64;
        for candidate in candidates {
            let source_rows =
                usize::try_from(candidate.row_count).map_err(|_| ProcessorError::InvalidNumeric)?;
            if row_count.saturating_add(source_rows) > self.configuration.maximum_compacted_rows {
                break;
            }
            let candidate_min_server = parse_u64(&candidate.min_server)?;
            let candidate_max_server = parse_u64(&candidate.max_server)?;
            let candidate_min_occurred = parse_u64(&candidate.min_occurred)?;
            let candidate_max_occurred = parse_u64(&candidate.max_occurred)?;
            min_server = min_server.min(candidate_min_server);
            max_server = max_server.max(candidate_max_server);
            min_occurred = min_occurred.min(candidate_min_occurred);
            max_occurred = max_occurred.max(candidate_max_occurred);
            row_count += source_rows;
            source_ids.push(candidate.batch_id);
        }
        if source_ids.len() < self.configuration.compaction_minimum {
            return Ok(None);
        }
        let partition_key = format!(
            "{organization}/{}/{}/{}/{}/{}",
            partition.project_id,
            partition.environment_id,
            partition.partition_day,
            partition.partition_hour,
            partition.envelope_kind
        );
        let batch = Batch {
            id: deterministic_batch_id("compaction", &partition_key, &[], &source_ids),
            organization_id: organization.to_owned(),
            project_id: partition.project_id,
            environment_id: partition.environment_id,
            kind: "compaction".to_owned(),
            partition_day: partition.partition_day,
            partition_hour: u8::try_from(partition.partition_hour)
                .map_err(|_| ProcessorError::InvalidNumeric)?,
            envelope_kind: partition.envelope_kind,
            row_count,
            min_server_received_at_unix_nano: min_server,
            max_server_received_at_unix_nano: max_server,
            min_effective_occurred_at_unix_nano: min_occurred,
            max_effective_occurred_at_unix_nano: max_occurred,
            attempt_count: 1,
            supersedes: source_ids,
        };
        insert_batch(transaction, &batch, &self.configuration).await?;
        let mut ordinal = 0_i32;
        for source_id in &batch.supersedes {
            sqlx::query(
                r"INSERT INTO lake.compaction_sources (compaction_batch_id,source_batch_id,
                    organization_id,project_id,environment_id) VALUES ($1,$2,$3::uuid,$4::uuid,$5::uuid)",
            )
            .bind(&batch.id)
            .bind(source_id)
            .bind(&batch.organization_id)
            .bind(&batch.project_id)
            .bind(&batch.environment_id)
            .execute(&mut **transaction)
            .await?;
            let copied = sqlx::query(
                r"INSERT INTO lake.batch_records (batch_id,organization_id,project_id,
                    environment_id,canonical_envelope_id,row_ordinal)
                  SELECT $1,$2::uuid,$3::uuid,$4::uuid,canonical_envelope_id,
                    (row_number() OVER (ORDER BY canonical_envelope_id)-1+$6)::integer
                  FROM lake.batch_records WHERE batch_id=$5",
            )
            .bind(&batch.id)
            .bind(&batch.organization_id)
            .bind(&batch.project_id)
            .bind(&batch.environment_id)
            .bind(source_id)
            .bind(ordinal)
            .execute(&mut **transaction)
            .await?
            .rows_affected();
            ordinal = ordinal
                .checked_add(i32::try_from(copied).map_err(|_| ProcessorError::InvalidNumeric)?)
                .ok_or(ProcessorError::InvalidNumeric)?;
        }
        if usize::try_from(ordinal).map_err(|_| ProcessorError::InvalidNumeric)? != row_count {
            return Err(ProcessorError::InvalidNumeric);
        }
        Ok(Some(batch))
    }

    async fn publish(&self, batch: &Batch) -> Result<(), ProcessorError> {
        let rows = self.load_rows(batch).await?;
        let parquet = encode_parquet(batch, &rows)?;
        let object_digest: [u8; 32] = Sha256::digest(&parquet).into();
        let object_key = batch.object_key()?;
        self.objects
            .put_if_absent(&object_key, Bytes::from(parquet.clone()), object_digest)
            .await?;
        let byte_count =
            i64::try_from(parquet.len()).map_err(|_| ProcessorError::InvalidNumeric)?;
        let manifest = encode_manifest(batch, &object_key, object_digest, byte_count)?;
        let manifest_digest: [u8; 32] = Sha256::digest(&manifest).into();
        let manifest_key = batch.manifest_key()?;
        self.objects
            .put_if_absent(&manifest_key, Bytes::from(manifest), manifest_digest)
            .await?;
        self.commit(
            batch,
            &Published {
                object_key,
                manifest_key,
                object_digest,
                manifest_digest,
                byte_count,
            },
        )
        .await
    }

    async fn load_rows(&self, batch: &Batch) -> Result<Vec<Row>, ProcessorError> {
        let mut transaction = self.control.begin_tenant(&batch.organization_id).await?;
        let rows = sqlx::query_as::<_, CanonicalRow>(LOAD_ROWS_SQL)
            .bind(&batch.id)
            .fetch_all(&mut *transaction)
            .await?;
        transaction.commit().await?;
        rows.into_iter().map(CanonicalRow::into_row).collect()
    }

    async fn commit(&self, batch: &Batch, published: &Published) -> Result<(), ProcessorError> {
        let mut transaction = self.control.begin_tenant(&batch.organization_id).await?;
        let affected = sqlx::query(
            r"UPDATE lake.export_batches SET status='committed',object_key=$1,manifest_key=$2,
                object_sha256=$3,manifest_sha256=$4,byte_count=$5,published_at=clock_timestamp(),
                lease_owner=NULL,lease_expires_at=NULL,last_error_code=NULL,last_error_message=NULL
                WHERE batch_id=$6 AND status='leased' AND lease_owner=$7",
        )
        .bind(&published.object_key)
        .bind(&published.manifest_key)
        .bind(published.object_digest.as_slice())
        .bind(published.manifest_digest.as_slice())
        .bind(published.byte_count)
        .bind(&batch.id)
        .bind(&self.configuration.worker_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(ProcessorError::LostLease);
        }
        if batch.kind == "compaction" {
            let superseded = sqlx::query(
                r"UPDATE lake.export_batches AS source SET status='superseded',
                    superseded_at=clock_timestamp() FROM lake.compaction_sources AS relation
                  WHERE relation.compaction_batch_id=$1 AND source.batch_id=relation.source_batch_id
                    AND source.status='committed'",
            )
            .bind(&batch.id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if usize::try_from(superseded).map_err(|_| ProcessorError::InvalidNumeric)?
                != batch.supersedes.len()
            {
                return Err(ProcessorError::LostLease);
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn handle_failure(
        &self,
        batch: &Batch,
        cause: &ProcessorError,
    ) -> Result<(), ProcessorError> {
        let terminal = matches!(
            cause,
            ProcessorError::Format(_) | ProcessorError::Object(ObjectError::ImmutableConflict)
        );
        let exhausted = batch.attempt_count >= self.configuration.maximum_attempts;
        let status = if terminal || exhausted {
            "dead_letter"
        } else {
            "pending"
        };
        let exponent = u32::try_from((batch.attempt_count - 1).clamp(0, 8)).unwrap_or_default();
        let delay = if status == "pending" {
            1_i32 << exponent
        } else {
            0
        };
        let code = if terminal {
            "lake.object_integrity"
        } else {
            "lake.publish"
        };
        let cause_text = cause.to_string();
        let message = truncate(&cause_text, 512);
        let mut transaction = self.control.begin_tenant(&batch.organization_id).await?;
        let affected = sqlx::query(
            r"UPDATE lake.export_batches SET status=$1,available_at=clock_timestamp()+make_interval(secs=>$2),
                lease_owner=NULL,lease_expires_at=NULL,last_error_code=$3,last_error_message=$4
                WHERE batch_id=$5 AND status='leased' AND lease_owner=$6",
        )
        .bind(status).bind(delay).bind(code).bind(message).bind(&batch.id)
        .bind(&self.configuration.worker_id).execute(&mut *transaction).await?.rows_affected();
        if affected != 1 {
            return Err(ProcessorError::LostLease);
        }
        transaction.commit().await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct BatchRow {
    batch_id: String,
    organization_id: String,
    project_id: String,
    environment_id: String,
    batch_kind: String,
    partition_day: Date,
    partition_hour: i32,
    envelope_kind: String,
    row_count: i32,
    min_server: String,
    max_server: String,
    min_occurred: String,
    max_occurred: String,
    attempt_count: i32,
}

impl BatchRow {
    fn into_batch(self) -> Result<Batch, ProcessorError> {
        Ok(Batch {
            id: self.batch_id,
            organization_id: self.organization_id,
            project_id: self.project_id,
            environment_id: self.environment_id,
            kind: self.batch_kind,
            partition_day: self.partition_day,
            partition_hour: u8::try_from(self.partition_hour)
                .map_err(|_| ProcessorError::InvalidNumeric)?,
            envelope_kind: self.envelope_kind,
            row_count: usize::try_from(self.row_count)
                .map_err(|_| ProcessorError::InvalidNumeric)?,
            min_server_received_at_unix_nano: parse_u64(&self.min_server)?,
            max_server_received_at_unix_nano: parse_u64(&self.max_server)?,
            min_effective_occurred_at_unix_nano: parse_u64(&self.min_occurred)?,
            max_effective_occurred_at_unix_nano: parse_u64(&self.max_occurred)?,
            attempt_count: self.attempt_count,
            supersedes: Vec::new(),
        })
    }
}

#[derive(sqlx::FromRow)]
struct CandidateRow {
    id: i64,
    project_id: String,
    environment_id: String,
    envelope_kind: String,
    record_digest: Vec<u8>,
    server_received_nano: String,
    effective_occurred: String,
    estimated_bytes: i32,
}

#[derive(sqlx::FromRow)]
struct CompactionPartition {
    project_id: String,
    environment_id: String,
    partition_day: Date,
    partition_hour: i32,
    envelope_kind: String,
}

#[derive(sqlx::FromRow)]
struct CompactionSource {
    batch_id: String,
    row_count: i32,
    min_server: String,
    max_server: String,
    min_occurred: String,
    max_occurred: String,
}

#[derive(sqlx::FromRow)]
struct CanonicalRow {
    id: i64,
    organization_id: String,
    project_id: String,
    environment_id: String,
    data_source_id: String,
    envelope_version: String,
    envelope_kind: String,
    record_id: String,
    record_digest: Vec<u8>,
    installation_id: Option<String>,
    session_id: Option<String>,
    replay_id: Option<String>,
    replay_chunk_id: Option<String>,
    trace_id: Option<String>,
    span_id: Option<String>,
    occurred: Option<String>,
    observed: Option<String>,
    server: String,
    effective: String,
    monotonic: Option<String>,
    boot_id: Option<String>,
    sequence: Option<String>,
    clock_skew: i64,
    timing_class: String,
    late_arrival: bool,
    canonical_json: String,
    normalized_at: OffsetDateTime,
}

impl CanonicalRow {
    fn into_row(self) -> Result<Row, ProcessorError> {
        let digest: [u8; 32] = self
            .record_digest
            .try_into()
            .map_err(|_| ProcessorError::InvalidNumeric)?;
        let normalized_at_unix_nano = i64::try_from(self.normalized_at.unix_timestamp_nanos())
            .map_err(|_| ProcessorError::InvalidNumeric)?;
        Ok(Row {
            dataset_schema_version: String::new(),
            batch_id: String::new(),
            row_ordinal: 0,
            canonical_envelope_id: self.id,
            organization_id: self.organization_id,
            project_id: self.project_id,
            environment_id: self.environment_id,
            data_source_id: self.data_source_id,
            envelope_version: self.envelope_version,
            envelope_kind: self.envelope_kind,
            record_id: self.record_id,
            record_sha256: hex::encode(digest),
            installation_id: self.installation_id,
            session_id: self.session_id,
            replay_id: self.replay_id,
            replay_chunk_id: self.replay_chunk_id,
            trace_id: self.trace_id,
            span_id: self.span_id,
            occurred_at_unix_nano: parse_optional(self.occurred)?,
            source_observed_at_unix_nano: parse_optional(self.observed)?,
            server_received_at_unix_nano: parse_u64(&self.server)?,
            effective_occurred_at_unix_nano: parse_u64(&self.effective)?,
            monotonic_nano: parse_optional(self.monotonic)?,
            boot_id: self.boot_id,
            sequence_number: parse_optional(self.sequence)?,
            clock_skew_nano: self.clock_skew,
            timing_class: self.timing_class,
            late_arrival: self.late_arrival,
            canonical_json: self.canonical_json,
            normalized_at_unix_nano,
        })
    }
}

async fn insert_batch(
    transaction: &mut Transaction<'_, Postgres>,
    batch: &Batch,
    configuration: &ProcessorConfiguration,
) -> Result<(), ProcessorError> {
    let seconds = i32::try_from(configuration.lease_duration.as_secs()).unwrap_or(i32::MAX);
    sqlx::query(r"INSERT INTO lake.export_batches (batch_id,organization_id,project_id,environment_id,batch_kind,
        partition_day,partition_hour,envelope_kind,status,attempt_count,lease_owner,lease_expires_at,row_count,
        min_server_received_at_unix_nano,max_server_received_at_unix_nano,min_effective_occurred_at_unix_nano,
        max_effective_occurred_at_unix_nano) VALUES ($1,$2::uuid,$3::uuid,$4::uuid,$5,$6,$7,$8,'leased',1,$9,
        clock_timestamp()+make_interval(secs=>$10),$11,$12::numeric,$13::numeric,$14::numeric,$15::numeric)")
        .bind(&batch.id).bind(&batch.organization_id).bind(&batch.project_id).bind(&batch.environment_id)
        .bind(&batch.kind).bind(batch.partition_day).bind(i32::from(batch.partition_hour)).bind(&batch.envelope_kind)
        .bind(&configuration.worker_id).bind(seconds).bind(i32::try_from(batch.row_count).map_err(|_|ProcessorError::InvalidNumeric)?)
        .bind(batch.min_server_received_at_unix_nano.to_string()).bind(batch.max_server_received_at_unix_nano.to_string())
        .bind(batch.min_effective_occurred_at_unix_nano.to_string()).bind(batch.max_effective_occurred_at_unix_nano.to_string())
        .execute(&mut **transaction).await?;
    Ok(())
}

fn parse_u64(value: &str) -> Result<u64, ProcessorError> {
    value.parse().map_err(|_| ProcessorError::InvalidNumeric)
}
fn parse_optional(value: Option<String>) -> Result<Option<u64>, ProcessorError> {
    value.map(|v| parse_u64(&v)).transpose()
}
fn truncate(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        return value;
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

const CANDIDATE_FIRST_SQL: &str = r"SELECT envelope.id,envelope.project_id::text,envelope.environment_id::text,
    envelope.envelope_kind,envelope.record_sha256 AS record_digest,envelope.server_received_at_unix_nano::text AS server_received_nano,
    envelope.effective_occurred_at_unix_nano::text AS effective_occurred,(pg_column_size(envelope.canonical)+512)::integer AS estimated_bytes
    FROM ingest.canonical_envelopes AS envelope WHERE NOT EXISTS (SELECT 1 FROM lake.source_claims AS claim
    WHERE claim.canonical_envelope_id=envelope.id) AND NOT EXISTS (SELECT 1 FROM lifecycle.deletion_requests AS request
    WHERE request.organization_id=envelope.organization_id AND request.status IN ('pending','leased') AND
    (request.kind='tenant' OR (request.project_id=envelope.project_id AND request.environment_id=envelope.environment_id)))
    ORDER BY envelope.id FOR UPDATE OF envelope SKIP LOCKED LIMIT 1";

const CANDIDATE_PARTITION_SQL: &str = r"SELECT envelope.id,envelope.project_id::text,envelope.environment_id::text,
    envelope.envelope_kind,envelope.record_sha256 AS record_digest,envelope.server_received_at_unix_nano::text AS server_received_nano,
    envelope.effective_occurred_at_unix_nano::text AS effective_occurred,(pg_column_size(envelope.canonical)+512)::integer AS estimated_bytes
    FROM ingest.canonical_envelopes AS envelope WHERE envelope.project_id=$1::uuid AND envelope.environment_id=$2::uuid
    AND envelope.envelope_kind=$3 AND envelope.server_received_at_unix_nano >= $4::numeric
    AND envelope.server_received_at_unix_nano < $5::numeric AND NOT EXISTS (SELECT 1 FROM lake.source_claims AS claim
    WHERE claim.canonical_envelope_id=envelope.id) ORDER BY envelope.id FOR UPDATE OF envelope SKIP LOCKED LIMIT $6";

const LOAD_ROWS_SQL: &str = r"SELECT envelope.id,envelope.organization_id::text,envelope.project_id::text,
    envelope.environment_id::text,envelope.data_source_id::text,envelope.envelope_version,envelope.envelope_kind,
    envelope.record_id,envelope.record_sha256 AS record_digest,envelope.installation_id,envelope.session_id,
    envelope.replay_id,envelope.replay_chunk_id,envelope.trace_id,envelope.span_id,
    envelope.occurred_at_unix_nano::text AS occurred,envelope.source_observed_at_unix_nano::text AS observed,
    envelope.server_received_at_unix_nano::text AS server,envelope.effective_occurred_at_unix_nano::text AS effective,
    envelope.monotonic_nano::text AS monotonic,envelope.boot_id,envelope.sequence_number::text AS sequence,
    envelope.clock_skew_nano::bigint AS clock_skew,envelope.timing_class,envelope.late_arrival,
    envelope.canonical::text AS canonical_json,envelope.normalized_at
    FROM lake.batch_records AS member JOIN ingest.canonical_envelopes AS envelope ON envelope.id=member.canonical_envelope_id
    WHERE member.batch_id=$1 ORDER BY member.row_ordinal";

const COMPACTION_PARTITION_SQL: &str = r"SELECT batch.project_id::text AS project_id,
    batch.environment_id::text AS environment_id,batch.partition_day,
    batch.partition_hour::integer AS partition_hour,batch.envelope_kind
    FROM lake.export_batches AS batch WHERE batch.status='committed' AND batch.row_count < $2
    AND NOT EXISTS (SELECT 1 FROM lifecycle.deletion_requests AS request
      WHERE request.organization_id=batch.organization_id AND request.status IN ('pending','leased')
      AND (request.kind='tenant' OR (request.project_id=batch.project_id AND request.environment_id=batch.environment_id)))
    AND NOT EXISTS (SELECT 1 FROM lake.compaction_sources AS source
      JOIN lake.export_batches AS compaction ON compaction.batch_id=source.compaction_batch_id
      WHERE source.source_batch_id=batch.batch_id AND compaction.status IN ('pending','leased','committed'))
    GROUP BY batch.project_id,batch.environment_id,batch.partition_day,batch.partition_hour,batch.envelope_kind
    HAVING count(*) >= $1 ORDER BY min(batch.published_at) LIMIT 1";

const COMPACTION_SOURCES_SQL: &str = r"SELECT batch.batch_id,batch.row_count,
    batch.min_server_received_at_unix_nano::text AS min_server,
    batch.max_server_received_at_unix_nano::text AS max_server,
    batch.min_effective_occurred_at_unix_nano::text AS min_occurred,
    batch.max_effective_occurred_at_unix_nano::text AS max_occurred
    FROM lake.export_batches AS batch WHERE batch.project_id=$1::uuid AND batch.environment_id=$2::uuid
    AND batch.partition_day=$3 AND batch.partition_hour=$4 AND batch.envelope_kind=$5
    AND batch.status='committed' AND batch.row_count < $7
    AND NOT EXISTS (SELECT 1 FROM lifecycle.deletion_requests AS request
      WHERE request.organization_id=batch.organization_id AND request.status IN ('pending','leased')
      AND (request.kind='tenant' OR (request.project_id=batch.project_id AND request.environment_id=batch.environment_id)))
    AND NOT EXISTS (SELECT 1 FROM lake.compaction_sources AS source
      JOIN lake.export_batches AS compaction ON compaction.batch_id=source.compaction_batch_id
      WHERE source.source_batch_id=batch.batch_id AND compaction.status IN ('pending','leased','committed'))
    ORDER BY batch.row_count,batch.published_at,batch.batch_id FOR UPDATE OF batch SKIP LOCKED LIMIT $6";
