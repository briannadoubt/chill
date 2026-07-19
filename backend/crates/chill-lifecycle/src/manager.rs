use std::{fmt::Write as _, fs, path::PathBuf, time::Duration};

use bytes::Bytes;
use chill_control_plane::{ControlPlaneError, Store};
use chill_lake::{
    Batch, LakeError, Row, decode_parquet, deterministic_batch_id, encode_manifest, encode_parquet,
};
use chill_objects::{ImmutableStore, ObjectError};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use time::{Date, OffsetDateTime};
use tokio::time::MissedTickBehavior;
use tracing::error;

use crate::{Completion, Kind, Request, RequestError, TargetKind};

/// Durable lifecycle worker bounds.
#[derive(Clone, Debug)]
pub struct Configuration {
    /// Stable lease owner.
    pub worker_id: String,
    /// Lease lifetime.
    pub lease_duration: Duration,
    /// Idle poll interval.
    pub poll_interval: Duration,
    /// Fixed retry delay.
    pub retry_delay: Duration,
    /// Attempts before terminal failure.
    pub maximum_attempts: i32,
    /// Maximum source lake files per request.
    pub maximum_files: usize,
    /// Digest-addressed local query cache root.
    pub query_cache_path: PathBuf,
}

impl Configuration {
    /// Returns production lifecycle defaults.
    #[must_use]
    pub fn production(worker_id: impl Into<String>) -> Self {
        Self {
            worker_id: worker_id.into(),
            lease_duration: Duration::from_mins(5),
            poll_interval: Duration::from_secs(5),
            retry_delay: Duration::from_secs(5),
            maximum_attempts: 8,
            maximum_files: 10_000,
            query_cache_path: PathBuf::from(".chill-data/query-cache"),
        }
    }

    fn validate(&self) -> Result<(), LifecycleError> {
        if self.worker_id.is_empty()
            || self.worker_id.len() > 128
            || self.lease_duration.is_zero()
            || self.poll_interval.is_zero()
            || self.retry_delay.is_zero()
            || self.maximum_attempts < 1
            || self.maximum_files == 0
            || self.query_cache_path.as_os_str().is_empty()
        {
            return Err(LifecycleError::InvalidConfiguration);
        }
        Ok(())
    }
}

/// Lifecycle request or worker failure.
#[derive(Debug, Error)]
pub enum LifecycleError {
    /// Dependencies or resource bounds are invalid.
    #[error("lifecycle manager configuration is invalid")]
    InvalidConfiguration,
    /// Public request validation failed.
    #[error("invalid lifecycle request: {0}")]
    Request(#[from] RequestError),
    /// Tenant transaction setup failed.
    #[error("lifecycle tenant operation failed: {0}")]
    Control(#[from] ControlPlaneError),
    /// Database operation failed.
    #[error("lifecycle database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// Immutable storage failed.
    #[error("lifecycle object operation failed: {0}")]
    Object(#[from] ObjectError),
    /// Parquet or manifest processing failed.
    #[error("lifecycle lake operation failed: {0}")]
    Lake(#[from] LakeError),
    /// Durable state violated an invariant.
    #[error("lifecycle state is invalid: {0}")]
    State(String),
    /// Request touches too many immutable files.
    #[error("lifecycle request exceeds its file bound")]
    FileLimit,
    /// Worker lost ownership before transition.
    #[error("lifecycle worker lost its request lease")]
    LostLease,
}

/// Retention scheduler and deletion/rewrite worker.
#[derive(Clone)]
pub struct Manager {
    pool: PgPool,
    control: Store,
    objects: ImmutableStore,
    configuration: Configuration,
}

impl Manager {
    /// Creates a lifecycle manager.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe worker bounds.
    pub fn new(
        pool: PgPool,
        control: Store,
        objects: ImmutableStore,
        configuration: Configuration,
    ) -> Result<Self, LifecycleError> {
        configuration.validate()?;
        Ok(Self {
            pool,
            control,
            objects,
            configuration,
        })
    }

    /// Creates an idempotent deletion, immediately tombstones subject IDs, and
    /// suspends credentials for scope deletion.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid input, idempotency mismatch, or durable
    /// transaction failure.
    #[allow(
        clippy::too_many_lines,
        reason = "request, tombstone, suspension, and audit must commit atomically"
    )]
    pub async fn create_request(&self, mut request: Request) -> Result<Request, LifecycleError> {
        request.validate()?;
        request.target_sha256 = request.target_digest();
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let target_kind = request.target_kind.map(TargetKind::as_str);
        let target_value =
            (request.kind == Kind::DataSubject).then_some(request.target_value.as_str());
        let target_digest =
            (request.kind == Kind::DataSubject).then_some(request.target_sha256.as_slice());
        let cutoff = request.cutoff_unix_nano.map(|value| value.to_string());
        let created = sqlx::query(
            r"INSERT INTO lifecycle.deletion_requests (organization_id,project_id,environment_id,
              kind,target_kind,target_value,target_sha256,cutoff_unix_nano,requested_by,reason_code,
              idempotency_key) VALUES ($1::uuid,$2::uuid,$3::uuid,$4,$5,$6,$7,$8::numeric,$9,$10,$11)
              ON CONFLICT (organization_id,idempotency_key) DO NOTHING",
        )
        .bind(&request.organization_id)
        .bind(nonempty(&request.project_id))
        .bind(nonempty(&request.environment_id))
        .bind(request.kind.as_str())
        .bind(target_kind)
        .bind(target_value)
        .bind(target_digest)
        .bind(cutoff)
        .bind(&request.requested_by)
        .bind(&request.reason_code)
        .bind(&request.idempotency_key)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        let persisted = request_by_key(
            &mut transaction,
            &request.organization_id,
            &request.idempotency_key,
        )
        .await?;
        if !same_request(&request, &persisted) {
            return Err(LifecycleError::State(
                "deletion idempotency key was reused with different input".to_owned(),
            ));
        }
        if request.kind == Kind::DataSubject {
            sqlx::query(
                r"INSERT INTO lifecycle.subject_tombstones (request_id,organization_id,project_id,
                  environment_id,target_kind,target_sha256) VALUES ($1::uuid,$2::uuid,$3::uuid,$4::uuid,$5,$6)
                  ON CONFLICT (environment_id,target_kind,target_sha256) DO NOTHING",
            )
            .bind(&persisted.id)
            .bind(&persisted.organization_id)
            .bind(&persisted.project_id)
            .bind(&persisted.environment_id)
            .bind(target_kind)
            .bind(persisted.target_sha256.as_slice())
            .execute(&mut *transaction)
            .await?;
        }
        if matches!(request.kind, Kind::Environment | Kind::Tenant) {
            suspend_scope(&mut transaction, &request).await?;
        }
        if created {
            sqlx::query(
                r"INSERT INTO control.audit_log (organization_id,action,target_type,target_id,
                  request_id,reason_code,details) VALUES ($1::uuid,'lifecycle.deletion.requested',
                  'deletion_request',$2::uuid,$3,$4,jsonb_build_object('kind',$5::text,
                  'project_id',$6::text,'environment_id',$7::text)) ON CONFLICT DO NOTHING",
            )
            .bind(&persisted.organization_id)
            .bind(&persisted.id)
            .bind(&persisted.idempotency_key)
            .bind(&persisted.reason_code)
            .bind(persisted.kind.as_str())
            .bind(nonempty(&persisted.project_id))
            .bind(nonempty(&persisted.environment_id))
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(persisted)
    }

    /// Schedules daily environment and replay retention requests.
    ///
    /// # Errors
    ///
    /// Returns an error when scopes cannot be listed or a request cannot be
    /// durably created.
    pub async fn schedule_due_retention(
        &self,
        now: OffsetDateTime,
    ) -> Result<usize, LifecycleError> {
        let scopes = sqlx::query_as::<_, RetentionScope>(
            r"SELECT organization_id::text,project_id::text,environment_id::text,
              retention_days,replay_retention_days FROM control.list_retention_scopes()",
        )
        .fetch_all(&self.pool)
        .await?;
        let day = now.date();
        let mut created = 0;
        for scope in scopes {
            for (kind, days) in [
                (Kind::Retention, scope.retention_days),
                (Kind::ReplayExpiry, scope.replay_retention_days),
            ] {
                let cutoff = day
                    .midnight()
                    .assume_utc()
                    .checked_sub(time::Duration::days(i64::from(days)))
                    .ok_or_else(|| LifecycleError::State("retention cutoff overflow".to_owned()))?;
                let cutoff = u64::try_from(cutoff.unix_timestamp_nanos()).map_err(|_| {
                    LifecycleError::State("retention cutoff is negative".to_owned())
                })?;
                self.create_request(Request {
                    id: String::new(),
                    organization_id: scope.organization_id.clone(),
                    project_id: scope.project_id.clone(),
                    environment_id: scope.environment_id.clone(),
                    kind,
                    target_kind: None,
                    target_value: String::new(),
                    target_sha256: [0; 32],
                    cutoff_unix_nano: Some(cutoff),
                    requested_by: "lifecycle.scheduler".to_owned(),
                    reason_code: "retention.environment_policy".to_owned(),
                    idempotency_key: format!("{}:{day}:{}", kind.as_str(), scope.environment_id),
                    attempt_count: 0,
                    created_at: None,
                })
                .await?;
                created += 1;
            }
        }
        Ok(created)
    }

    /// Runs until cancelled.
    pub async fn run(&self) {
        let mut ticker = tokio::time::interval(self.configuration.poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            match self.process_available().await {
                Ok(0) => {
                    ticker.tick().await;
                }
                Ok(_) => {}
                Err(cause) => {
                    error!(%cause, "lifecycle pass failed");
                    ticker.tick().await;
                }
            }
        }
    }

    /// Processes at most one pending or expired-lease request.
    ///
    /// # Errors
    ///
    /// Returns listing, leasing, processing, or retry-transition failures.
    pub async fn process_available(&self) -> Result<usize, LifecycleError> {
        let organizations = sqlx::query_scalar::<_, String>(
            "SELECT organization_id::text FROM control.list_lifecycle_organization_ids() AS organization_id",
        )
        .fetch_all(&self.pool)
        .await?;
        for organization in organizations {
            let Some(request) = self.lease(&organization).await? else {
                continue;
            };
            if let Err(cause) = self.process(&request).await {
                self.handle_failure(&request, &cause).await?;
            }
            return Ok(1);
        }
        Ok(0)
    }

    async fn lease(&self, organization: &str) -> Result<Option<Request>, LifecycleError> {
        let seconds =
            i32::try_from(self.configuration.lease_duration.as_secs()).unwrap_or(i32::MAX);
        let mut transaction = self.control.begin_tenant(organization).await?;
        let row = sqlx::query_as::<_, RequestRow>(
            r"WITH candidate AS (SELECT id FROM lifecycle.deletion_requests WHERE
              (status='pending' AND available_at<=clock_timestamp()) OR
              (status='leased' AND lease_expires_at<=clock_timestamp())
              ORDER BY available_at,created_at,id FOR UPDATE SKIP LOCKED LIMIT 1)
              UPDATE lifecycle.deletion_requests AS request SET status='leased',lease_owner=$1,
              lease_expires_at=clock_timestamp()+make_interval(secs=>$2),
              attempt_count=request.attempt_count+1,started_at=coalesce(request.started_at,clock_timestamp()),
              last_error_code=NULL,last_error_message=NULL FROM candidate WHERE request.id=candidate.id
              RETURNING request.id::text,request.organization_id::text,request.project_id::text,
              request.environment_id::text,request.kind,request.target_kind,request.target_value,
              request.target_sha256,request.cutoff_unix_nano::text AS cutoff,request.requested_by,
              request.reason_code,request.idempotency_key,request.attempt_count,request.created_at",
        )
        .bind(&self.configuration.worker_id)
        .bind(seconds)
        .fetch_optional(&mut *transaction)
        .await?;
        transaction.commit().await?;
        row.map(RequestRow::into_request).transpose()
    }

    async fn process(&self, request: &Request) -> Result<Completion, LifecycleError> {
        self.prepare_lake(request).await?;
        self.rewrite_affected_batches(request).await?;
        self.purge_postgres(request).await?;
        self.delete_objects(request).await?;
        self.complete(request).await
    }

    async fn handle_failure(
        &self,
        request: &Request,
        cause: &LifecycleError,
    ) -> Result<(), LifecycleError> {
        let status = if request.attempt_count >= self.configuration.maximum_attempts {
            "failed"
        } else {
            "pending"
        };
        let seconds = i32::try_from(self.configuration.retry_delay.as_secs()).unwrap_or(i32::MAX);
        let message = truncate(&cause.to_string(), 512).to_owned();
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let affected = sqlx::query(
            r"UPDATE lifecycle.deletion_requests SET status=$1,
              available_at=clock_timestamp()+make_interval(secs=>$2),lease_owner=NULL,
              lease_expires_at=NULL,last_error_code='lifecycle.process',last_error_message=$3
              WHERE id=$4::uuid AND status='leased' AND lease_owner=$5",
        )
        .bind(status)
        .bind(seconds)
        .bind(message)
        .bind(&request.id)
        .bind(&self.configuration.worker_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(LifecycleError::LostLease);
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn prepare_lake(&self, request: &Request) -> Result<(), LifecycleError> {
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let active: i64 = if request.kind == Kind::Tenant {
            sqlx::query_scalar("SELECT count(*) FROM lake.export_batches WHERE organization_id=$1::uuid AND status='leased' AND lease_expires_at>clock_timestamp()")
                .bind(&request.organization_id).fetch_one(&mut *transaction).await?
        } else {
            sqlx::query_scalar("SELECT count(*) FROM lake.export_batches WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND status='leased' AND lease_expires_at>clock_timestamp()")
                .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
                .fetch_one(&mut *transaction).await?
        };
        if active != 0 {
            return Err(LifecycleError::State(format!(
                "{active} lake publication leases remain active"
            )));
        }
        let batches: Vec<String> = if request.kind == Kind::Tenant {
            sqlx::query_scalar("SELECT batch_id FROM lake.export_batches WHERE organization_id=$1::uuid AND (status='pending' OR (status='leased' AND lease_expires_at<=clock_timestamp())) FOR UPDATE")
                .bind(&request.organization_id).fetch_all(&mut *transaction).await?
        } else {
            sqlx::query_scalar("SELECT batch_id FROM lake.export_batches WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND (status='pending' OR (status='leased' AND lease_expires_at<=clock_timestamp())) FOR UPDATE")
                .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
                .fetch_all(&mut *transaction).await?
        };
        for batch in batches {
            sqlx::query("DELETE FROM lake.source_claims WHERE batch_id=$1")
                .bind(&batch)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM lake.batch_records WHERE batch_id=$1")
                .bind(&batch)
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                r"UPDATE lake.export_batches SET status='dead_letter',lease_owner=NULL,
                  lease_expires_at=NULL,last_error_code='lifecycle.cancelled',
                  last_error_message='cancelled before publication by an active lifecycle deletion'
                  WHERE batch_id=$1",
            )
            .bind(batch)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn rewrite_affected_batches(&self, request: &Request) -> Result<(), LifecycleError> {
        let sources = self.source_batches(request).await?;
        for source in sources {
            let body = self.objects.get(&source.object_key).await?;
            if i64::try_from(body.len()).ok() != Some(source.byte_count)
                || Sha256::digest(&body).as_slice() != source.object_digest
            {
                return Err(LifecycleError::State(format!(
                    "source {} failed byte or digest verification",
                    source.batch.id
                )));
            }
            let rows = decode_parquet(body)?;
            if rows.len() != source.batch.row_count {
                return Err(LifecycleError::State(format!(
                    "source {} row count changed",
                    source.batch.id
                )));
            }
            let mut survivors = Vec::new();
            let mut removed = 0_usize;
            for row in rows {
                if row.batch_id != source.batch.id
                    || row.organization_id != source.batch.organization_id
                    || row.project_id != source.batch.project_id
                    || row.environment_id != source.batch.environment_id
                {
                    return Err(LifecycleError::State(
                        "lake source contains a row outside its ledger scope".to_owned(),
                    ));
                }
                if request.matches(&row) {
                    removed += 1;
                } else {
                    survivors.push(row);
                }
            }
            if removed == 0 {
                continue;
            }
            let replacement = if source.status == "committed" && !survivors.is_empty() {
                Some(
                    self.publish_replacement(request, &source, &survivors)
                        .await?,
                )
            } else {
                None
            };
            self.commit_rewrite(request, &source, replacement.as_ref(), &survivors, removed)
                .await?;
        }
        Ok(())
    }

    async fn source_batches(&self, request: &Request) -> Result<Vec<SourceBatch>, LifecycleError> {
        let mut query = String::from(
            r"SELECT batch_id,organization_id::text,project_id::text,environment_id::text,
              batch_kind,partition_day,partition_hour::integer AS partition_hour,envelope_kind,row_count,
              min_server_received_at_unix_nano::text AS min_server,
              max_server_received_at_unix_nano::text AS max_server,
              min_effective_occurred_at_unix_nano::text AS min_occurred,
              max_effective_occurred_at_unix_nano::text AS max_occurred,status,object_key,
              manifest_key,object_sha256,manifest_sha256,byte_count FROM lake.export_batches
              WHERE organization_id=$1::uuid AND status IN ('committed','superseded')",
        );
        let mut next = 2;
        if request.kind != Kind::Tenant {
            query.push_str(" AND project_id=$2::uuid AND environment_id=$3::uuid");
            next = 4;
        }
        if request.kind == Kind::ReplayExpiry {
            query.push_str(" AND envelope_kind='replay'");
        }
        if matches!(request.kind, Kind::Retention | Kind::ReplayExpiry) {
            write!(
                query,
                " AND (status='superseded' OR min_effective_occurred_at_unix_nano < ${next}::numeric)"
            )
            .map_err(|_| LifecycleError::State("build source query failed".to_owned()))?;
            next += 1;
        }
        write!(query, " ORDER BY published_at,batch_id LIMIT ${next}")
            .map_err(|_| LifecycleError::State("build source query failed".to_owned()))?;
        // Appended SQL fragments are selected only from the closed Kind enum;
        // every request value remains a bind parameter.
        let mut statement = sqlx::query_as::<_, SourceRow>(sqlx::AssertSqlSafe(query.as_str()))
            .bind(&request.organization_id);
        if request.kind != Kind::Tenant {
            statement = statement
                .bind(&request.project_id)
                .bind(&request.environment_id);
        }
        if matches!(request.kind, Kind::Retention | Kind::ReplayExpiry) {
            statement = statement.bind(
                request
                    .cutoff_unix_nano
                    .ok_or_else(|| LifecycleError::State("retention cutoff missing".to_owned()))?
                    .to_string(),
            );
        }
        let limit = i64::try_from(self.configuration.maximum_files)
            .unwrap_or(i64::MAX)
            .saturating_add(1);
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let rows = statement.bind(limit).fetch_all(&mut *transaction).await?;
        transaction.commit().await?;
        if rows.len() > self.configuration.maximum_files {
            return Err(LifecycleError::FileLimit);
        }
        rows.into_iter().map(SourceRow::into_source).collect()
    }

    async fn publish_replacement(
        &self,
        request: &Request,
        source: &SourceBatch,
        rows: &[Row],
    ) -> Result<PublishedReplacement, LifecycleError> {
        let record_digests: Vec<Vec<u8>> = rows
            .iter()
            .map(|row| {
                let digest = hex::decode(&row.record_sha256).map_err(|_| {
                    LifecycleError::State("replacement record digest is invalid".to_owned())
                })?;
                if digest.len() != 32 {
                    return Err(LifecycleError::State(
                        "replacement record digest length is invalid".to_owned(),
                    ));
                }
                Ok(digest)
            })
            .collect::<Result<_, _>>()?;
        let min_server = rows
            .iter()
            .map(|row| row.server_received_at_unix_nano)
            .min()
            .ok_or_else(|| LifecycleError::State("replacement is empty".to_owned()))?;
        let max_server = rows
            .iter()
            .map(|row| row.server_received_at_unix_nano)
            .max()
            .ok_or_else(|| LifecycleError::State("replacement is empty".to_owned()))?;
        let min_occurred = rows
            .iter()
            .map(|row| row.effective_occurred_at_unix_nano)
            .min()
            .ok_or_else(|| LifecycleError::State("replacement is empty".to_owned()))?;
        let max_occurred = rows
            .iter()
            .map(|row| row.effective_occurred_at_unix_nano)
            .max()
            .ok_or_else(|| LifecycleError::State("replacement is empty".to_owned()))?;
        let partition_key = format!(
            "{}/{}/{}/{}/{}/{}/deletion={}",
            source.batch.organization_id,
            source.batch.project_id,
            source.batch.environment_id,
            source.batch.partition_day,
            source.batch.partition_hour,
            source.batch.envelope_kind,
            request.id
        );
        let batch = Batch {
            id: deterministic_batch_id(
                "rewrite",
                &partition_key,
                &record_digests,
                std::slice::from_ref(&source.batch.id),
            ),
            organization_id: source.batch.organization_id.clone(),
            project_id: source.batch.project_id.clone(),
            environment_id: source.batch.environment_id.clone(),
            kind: "rewrite".to_owned(),
            partition_day: source.batch.partition_day,
            partition_hour: source.batch.partition_hour,
            envelope_kind: source.batch.envelope_kind.clone(),
            row_count: rows.len(),
            min_server_received_at_unix_nano: min_server,
            max_server_received_at_unix_nano: max_server,
            min_effective_occurred_at_unix_nano: min_occurred,
            max_effective_occurred_at_unix_nano: max_occurred,
            attempt_count: 1,
            supersedes: vec![source.batch.id.clone()],
        };
        let body = encode_parquet(&batch, rows)?;
        let object_digest: [u8; 32] = Sha256::digest(&body).into();
        let object_key = batch.object_key()?;
        self.objects
            .put_if_absent(&object_key, Bytes::from(body.clone()), object_digest)
            .await?;
        let byte_count = i64::try_from(body.len())
            .map_err(|_| LifecycleError::State("replacement is too large".to_owned()))?;
        let manifest = encode_manifest(&batch, &object_key, object_digest, byte_count)?;
        let manifest_digest: [u8; 32] = Sha256::digest(&manifest).into();
        let manifest_key = batch.manifest_key()?;
        self.objects
            .put_if_absent(&manifest_key, Bytes::from(manifest), manifest_digest)
            .await?;
        Ok(PublishedReplacement {
            batch,
            object_key,
            manifest_key,
            object_digest,
            manifest_digest,
            byte_count,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "replacement registration and source tombstone are one atomic privacy boundary"
    )]
    async fn commit_rewrite(
        &self,
        request: &Request,
        source: &SourceBatch,
        replacement: Option<&PublishedReplacement>,
        survivors: &[Row],
        removed: usize,
    ) -> Result<(), LifecycleError> {
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let current = sqlx::query_as::<_, (String, Vec<u8>)>(
            "SELECT status,object_sha256 FROM lake.export_batches WHERE batch_id=$1 FOR UPDATE",
        )
        .bind(&source.batch.id)
        .fetch_one(&mut *transaction)
        .await?;
        if current.0 != source.status || current.1.as_slice() != source.object_digest {
            return Err(LifecycleError::State(
                "deletion source changed before rewrite commit".to_owned(),
            ));
        }
        if let Some(replacement) = replacement {
            let batch = &replacement.batch;
            sqlx::query(
                r"INSERT INTO lake.export_batches (batch_id,organization_id,project_id,environment_id,
                  batch_kind,partition_day,partition_hour,envelope_kind,status,attempt_count,object_key,
                  manifest_key,object_sha256,manifest_sha256,byte_count,row_count,
                  min_server_received_at_unix_nano,max_server_received_at_unix_nano,
                  min_effective_occurred_at_unix_nano,max_effective_occurred_at_unix_nano,published_at)
                  VALUES ($1,$2::uuid,$3::uuid,$4::uuid,'rewrite',$5,$6,$7,'committed',1,$8,$9,$10,$11,
                  $12,$13,$14::numeric,$15::numeric,$16::numeric,$17::numeric,clock_timestamp())",
            )
            .bind(&batch.id).bind(&batch.organization_id).bind(&batch.project_id).bind(&batch.environment_id)
            .bind(batch.partition_day).bind(i32::from(batch.partition_hour)).bind(&batch.envelope_kind)
            .bind(&replacement.object_key).bind(&replacement.manifest_key)
            .bind(replacement.object_digest.as_slice()).bind(replacement.manifest_digest.as_slice())
            .bind(replacement.byte_count).bind(i32::try_from(batch.row_count).map_err(|_|LifecycleError::State("row count overflow".to_owned()))?)
            .bind(batch.min_server_received_at_unix_nano.to_string()).bind(batch.max_server_received_at_unix_nano.to_string())
            .bind(batch.min_effective_occurred_at_unix_nano.to_string()).bind(batch.max_effective_occurred_at_unix_nano.to_string())
            .execute(&mut *transaction).await?;
            for (ordinal, row) in survivors.iter().enumerate() {
                sqlx::query("INSERT INTO lake.batch_records (batch_id,organization_id,project_id,environment_id,canonical_envelope_id,row_ordinal) VALUES ($1,$2::uuid,$3::uuid,$4::uuid,$5,$6)")
                    .bind(&batch.id).bind(&batch.organization_id).bind(&batch.project_id).bind(&batch.environment_id)
                    .bind(row.canonical_envelope_id).bind(i32::try_from(ordinal).map_err(|_|LifecycleError::State("ordinal overflow".to_owned()))?)
                    .execute(&mut *transaction).await?;
            }
            sqlx::query("INSERT INTO lake.rewrite_sources (rewrite_batch_id,source_batch_id,request_id,organization_id,project_id,environment_id) VALUES ($1,$2,$3::uuid,$4::uuid,$5::uuid,$6::uuid)")
                .bind(&batch.id).bind(&source.batch.id).bind(&request.id).bind(&batch.organization_id)
                .bind(&batch.project_id).bind(&batch.environment_id).execute(&mut *transaction).await?;
        }
        let affected = sqlx::query("UPDATE lake.export_batches SET status='tombstoned',tombstoned_at=clock_timestamp(),deletion_request_id=$1::uuid WHERE batch_id=$2 AND status=$3")
            .bind(&request.id).bind(&source.batch.id).bind(&source.status).execute(&mut *transaction).await?.rows_affected();
        if affected != 1 {
            return Err(LifecycleError::State(
                "source changed before tombstone".to_owned(),
            ));
        }
        sqlx::query("DELETE FROM lake.batch_records WHERE batch_id=$1")
            .bind(&source.batch.id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO lifecycle.deletion_batches (request_id,organization_id,project_id,environment_id,source_batch_id,replacement_batch_id,removed_rows,surviving_rows) VALUES ($1::uuid,$2::uuid,$3::uuid,$4::uuid,$5,$6,$7,$8)")
            .bind(&request.id).bind(&source.batch.organization_id).bind(&source.batch.project_id).bind(&source.batch.environment_id)
            .bind(&source.batch.id).bind(replacement.map(|value|value.batch.id.as_str()))
            .bind(i32::try_from(removed).map_err(|_|LifecycleError::State("removed rows overflow".to_owned()))?)
            .bind(i32::try_from(survivors.len()).map_err(|_|LifecycleError::State("survivor rows overflow".to_owned()))?)
            .execute(&mut *transaction).await?;
        for (kind, key, digest, bytes) in [
            (
                "parquet",
                source.object_key.as_str(),
                source.object_digest.as_slice(),
                source.byte_count,
            ),
            (
                "manifest",
                source.manifest_key.as_str(),
                source.manifest_digest.as_slice(),
                0,
            ),
        ] {
            sqlx::query("INSERT INTO lifecycle.deletion_objects (request_id,organization_id,project_id,environment_id,batch_id,object_kind,object_key,object_sha256,byte_count) VALUES ($1::uuid,$2::uuid,$3::uuid,$4::uuid,$5,$6,$7,$8,$9)")
                .bind(&request.id).bind(&source.batch.organization_id).bind(&source.batch.project_id).bind(&source.batch.environment_id)
                .bind(&source.batch.id).bind(kind).bind(key).bind(digest).bind(bytes).execute(&mut *transaction).await?;
        }
        if replacement.is_some() {
            sqlx::query("UPDATE lifecycle.deletion_requests SET rewritten_files=rewritten_files+1 WHERE id=$1::uuid")
                .bind(&request.id).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn purge_postgres(&self, request: &Request) -> Result<(), LifecycleError> {
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let ids = matching_canonical_ids(&mut transaction, request).await?;
        if !ids.is_empty() {
            let committed: i64 = sqlx::query_scalar(
                r"SELECT count(*) FROM lake.batch_records AS member
                  JOIN lake.export_batches AS batch ON batch.batch_id=member.batch_id
                  WHERE batch.status='committed' AND member.canonical_envelope_id=ANY($1)",
            )
            .bind(&ids)
            .fetch_one(&mut *transaction)
            .await?;
            if committed != 0 {
                return Err(LifecycleError::State(format!(
                    "{committed} target rows remain in committed lake files"
                )));
            }
        }
        let inbox = if ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query_as::<_, (i64, i64)>(
                r"SELECT DISTINCT inbox.id,octet_length(inbox.payload)::bigint FROM ingest.inbox
                  AS inbox JOIN ingest.canonical_envelopes AS envelope ON envelope.inbox_id=inbox.id
                  WHERE envelope.id=ANY($1)",
            )
            .bind(&ids)
            .fetch_all(&mut *transaction)
            .await?
        };
        if !ids.is_empty() {
            sqlx::query("DELETE FROM lake.batch_records WHERE canonical_envelope_id=ANY($1)")
                .bind(&ids)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM lake.source_claims WHERE canonical_envelope_id=ANY($1)")
                .bind(&ids)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM ingest.canonical_envelopes WHERE id=ANY($1)")
                .bind(&ids)
                .execute(&mut *transaction)
                .await?;
        }
        let mut inbox_bytes = 0_i64;
        for (id, bytes) in inbox {
            inbox_bytes = inbox_bytes.saturating_add(bytes);
            sqlx::query(
                r"UPDATE ingest.inbox SET payload=$1,metadata=jsonb_build_object(
                  'payload_disposition','lifecycle_erased','request_id',$2::text) WHERE id=$3",
            )
            .bind(b"lifecycle-erased".as_slice())
            .bind(&request.id)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                r"DELETE FROM ingest.inbox AS inbox WHERE id=$1 AND NOT EXISTS
                  (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)",
            )
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        }
        inbox_bytes =
            inbox_bytes.saturating_add(purge_unreferenced_inbox(&mut transaction, request).await?);
        purge_operational_buckets(&mut transaction, request).await?;
        sqlx::query(
            r"UPDATE lifecycle.deletion_requests SET deleted_rows=deleted_rows+$1,
              deleted_inbox_bytes=deleted_inbox_bytes+$2 WHERE id=$3::uuid",
        )
        .bind(i64::try_from(ids.len()).unwrap_or(i64::MAX))
        .bind(inbox_bytes)
        .bind(&request.id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn delete_objects(&self, request: &Request) -> Result<(), LifecycleError> {
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let objects = sqlx::query_as::<_, DeletionObject>(
            r"SELECT object_kind,object_key,object_sha256,byte_count,status
              FROM lifecycle.deletion_objects WHERE request_id=$1::uuid
              ORDER BY CASE object_kind WHEN 'manifest' THEN 0 ELSE 1 END,object_key",
        )
        .bind(&request.id)
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        for object in objects {
            if object.status == "deleted" {
                continue;
            }
            let expected: [u8; 32] = object
                .object_sha256
                .try_into()
                .map_err(|_| LifecycleError::State("queued object digest is invalid".to_owned()))?;
            let mut byte_count = object.byte_count;
            if let Ok(body) = self.objects.get(&object.object_key).await {
                if Sha256::digest(&body).as_slice() != expected {
                    return Err(LifecycleError::State(format!(
                        "deletion object {} failed digest verification",
                        object.object_key
                    )));
                }
                byte_count = i64::try_from(body.len()).unwrap_or(i64::MAX);
            }
            self.objects.delete(&object.object_key).await?;
            if object.object_kind == "parquet" {
                let cached = self
                    .configuration
                    .query_cache_path
                    .join(format!("{}.parquet", hex::encode(expected)));
                match fs::remove_file(cached) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(LifecycleError::State(format!(
                            "purge query cache file: {error}"
                        )));
                    }
                }
            }
            let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
            let affected = sqlx::query(
                r"UPDATE lifecycle.deletion_objects SET status='deleted',
                  deleted_at=clock_timestamp(),byte_count=$1,last_error_message=NULL
                  WHERE request_id=$2::uuid AND object_key=$3 AND status='pending'",
            )
            .bind(byte_count)
            .bind(&request.id)
            .bind(&object.object_key)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if affected == 1 {
                let deleted_files = i32::from(object.object_kind == "parquet");
                sqlx::query(
                    r"UPDATE lifecycle.deletion_requests SET
                      deleted_object_bytes=deleted_object_bytes+$1,
                      deleted_files=deleted_files+$2 WHERE id=$3::uuid",
                )
                .bind(byte_count)
                .bind(deleted_files)
                .bind(&request.id)
                .execute(&mut *transaction)
                .await?;
            }
            transaction.commit().await?;
        }
        Ok(())
    }

    async fn complete(&self, request: &Request) -> Result<Completion, LifecycleError> {
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let counters = sqlx::query_as::<_, (i64, i64, i64, i32, i32)>(
            r"SELECT deleted_rows,deleted_inbox_bytes,deleted_object_bytes,
              rewritten_files,deleted_files FROM lifecycle.deletion_requests
              WHERE id=$1::uuid FOR UPDATE",
        )
        .bind(&request.id)
        .fetch_one(&mut *transaction)
        .await?;
        let stable = serde_json::json!({
            "request_id": request.id,
            "kind": request.kind.as_str(),
            "deleted_rows": counters.0,
            "deleted_inbox_bytes": counters.1,
            "deleted_object_bytes": counters.2,
            "rewritten_files": counters.3,
            "deleted_files": counters.4,
        });
        let digest: [u8; 32] = Sha256::digest(serde_json::to_vec(&stable).map_err(|error| {
            LifecycleError::State(format!("completion serialization failed: {error}"))
        })?)
        .into();
        invalidate_derived(&mut transaction, request).await?;
        deactivate_scope(&mut transaction, request).await?;
        let affected = sqlx::query(
            r"UPDATE lifecycle.deletion_requests SET status='completed',lease_owner=NULL,
              lease_expires_at=NULL,target_value=NULL,completed_at=clock_timestamp(),
              completion_sha256=$1,last_error_code=NULL,last_error_message=NULL
              WHERE id=$2::uuid AND status='leased' AND lease_owner=$3",
        )
        .bind(digest.as_slice())
        .bind(&request.id)
        .bind(&self.configuration.worker_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(LifecycleError::LostLease);
        }
        let completion_sha256 = hex::encode(digest);
        sqlx::query(
            r"INSERT INTO control.audit_log (organization_id,action,target_type,target_id,
              request_id,reason_code,details) VALUES ($1::uuid,'lifecycle.deletion.completed',
              'deletion_request',$2::uuid,$3,$4,jsonb_build_object('kind',$5::text,
              'deleted_rows',$6::bigint,'deleted_inbox_bytes',$7::bigint,
              'deleted_object_bytes',$8::bigint,'rewritten_files',$9::integer,
              'deleted_files',$10::integer,'completion_sha256',$11::text))",
        )
        .bind(&request.organization_id)
        .bind(&request.id)
        .bind(&request.idempotency_key)
        .bind(&request.reason_code)
        .bind(request.kind.as_str())
        .bind(counters.0)
        .bind(counters.1)
        .bind(counters.2)
        .bind(counters.3)
        .bind(counters.4)
        .bind(&completion_sha256)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Completion {
            request_id: request.id.clone(),
            deleted_rows: counters.0,
            deleted_inbox_bytes: counters.1,
            deleted_object_bytes: counters.2,
            rewritten_files: counters.3,
            deleted_files: counters.4,
            completion_sha256,
        })
    }
}

#[derive(sqlx::FromRow)]
struct DeletionObject {
    object_kind: String,
    object_key: String,
    object_sha256: Vec<u8>,
    byte_count: i64,
    status: String,
}

#[derive(sqlx::FromRow)]
struct SourceRow {
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
    status: String,
    object_key: String,
    manifest_key: String,
    object_sha256: Vec<u8>,
    manifest_sha256: Vec<u8>,
    byte_count: i64,
}

struct SourceBatch {
    batch: Batch,
    status: String,
    object_key: String,
    manifest_key: String,
    object_digest: [u8; 32],
    manifest_digest: [u8; 32],
    byte_count: i64,
}

impl SourceRow {
    fn into_source(self) -> Result<SourceBatch, LifecycleError> {
        if self.byte_count < 1 {
            return Err(LifecycleError::State(
                "source byte count is invalid".to_owned(),
            ));
        }
        Ok(SourceBatch {
            batch: Batch {
                id: self.batch_id,
                organization_id: self.organization_id,
                project_id: self.project_id,
                environment_id: self.environment_id,
                kind: self.batch_kind,
                partition_day: self.partition_day,
                partition_hour: u8::try_from(self.partition_hour)
                    .map_err(|_| LifecycleError::State("partition hour is invalid".to_owned()))?,
                envelope_kind: self.envelope_kind,
                row_count: usize::try_from(self.row_count)
                    .map_err(|_| LifecycleError::State("row count is invalid".to_owned()))?,
                min_server_received_at_unix_nano: parse_u64(&self.min_server)?,
                max_server_received_at_unix_nano: parse_u64(&self.max_server)?,
                min_effective_occurred_at_unix_nano: parse_u64(&self.min_occurred)?,
                max_effective_occurred_at_unix_nano: parse_u64(&self.max_occurred)?,
                attempt_count: 1,
                supersedes: Vec::new(),
            },
            status: self.status,
            object_key: self.object_key,
            manifest_key: self.manifest_key,
            object_digest: self
                .object_sha256
                .try_into()
                .map_err(|_| LifecycleError::State("source digest is invalid".to_owned()))?,
            manifest_digest: self
                .manifest_sha256
                .try_into()
                .map_err(|_| LifecycleError::State("manifest digest is invalid".to_owned()))?,
            byte_count: self.byte_count,
        })
    }
}

struct PublishedReplacement {
    batch: Batch,
    object_key: String,
    manifest_key: String,
    object_digest: [u8; 32],
    manifest_digest: [u8; 32],
    byte_count: i64,
}

fn parse_u64(value: &str) -> Result<u64, LifecycleError> {
    value
        .parse()
        .map_err(|_| LifecycleError::State("numeric lake bound is invalid".to_owned()))
}

#[derive(sqlx::FromRow)]
struct RetentionScope {
    organization_id: String,
    project_id: String,
    environment_id: String,
    retention_days: i32,
    replay_retention_days: i32,
}

#[derive(sqlx::FromRow)]
struct RequestRow {
    id: String,
    organization_id: String,
    project_id: Option<String>,
    environment_id: Option<String>,
    kind: String,
    target_kind: Option<String>,
    target_value: Option<String>,
    target_sha256: Option<Vec<u8>>,
    cutoff: Option<String>,
    requested_by: String,
    reason_code: String,
    idempotency_key: String,
    attempt_count: i32,
    created_at: OffsetDateTime,
}

impl RequestRow {
    fn into_request(self) -> Result<Request, LifecycleError> {
        let kind = parse_kind(&self.kind)?;
        let target_kind = self.target_kind.as_deref().map(parse_target).transpose()?;
        let mut target_sha256 = [0; 32];
        if let Some(digest) = self.target_sha256 {
            target_sha256 = digest
                .try_into()
                .map_err(|_| LifecycleError::State("target digest length is invalid".to_owned()))?;
        }
        Ok(Request {
            id: self.id,
            organization_id: self.organization_id,
            project_id: self.project_id.unwrap_or_default(),
            environment_id: self.environment_id.unwrap_or_default(),
            kind,
            target_kind,
            target_value: self.target_value.unwrap_or_default(),
            target_sha256,
            cutoff_unix_nano: self
                .cutoff
                .map(|value| {
                    value
                        .parse()
                        .map_err(|_| LifecycleError::State("cutoff numeric is invalid".to_owned()))
                })
                .transpose()?,
            requested_by: self.requested_by,
            reason_code: self.reason_code,
            idempotency_key: self.idempotency_key,
            attempt_count: self.attempt_count,
            created_at: Some(self.created_at),
        })
    }
}

async fn request_by_key(
    transaction: &mut Transaction<'_, Postgres>,
    organization: &str,
    key: &str,
) -> Result<Request, LifecycleError> {
    sqlx::query_as::<_, RequestRow>(
        r"SELECT id::text,organization_id::text,project_id::text,environment_id::text,kind,
          target_kind,target_value,target_sha256,cutoff_unix_nano::text AS cutoff,requested_by,
          reason_code,idempotency_key,attempt_count,created_at FROM lifecycle.deletion_requests
          WHERE organization_id=$1::uuid AND idempotency_key=$2",
    )
    .bind(organization)
    .bind(key)
    .fetch_one(&mut **transaction)
    .await?
    .into_request()
}

fn parse_kind(value: &str) -> Result<Kind, LifecycleError> {
    match value {
        "retention" => Ok(Kind::Retention),
        "replay_expiry" => Ok(Kind::ReplayExpiry),
        "data_subject" => Ok(Kind::DataSubject),
        "environment" => Ok(Kind::Environment),
        "tenant" => Ok(Kind::Tenant),
        _ => Err(LifecycleError::State("deletion kind is invalid".to_owned())),
    }
}

fn parse_target(value: &str) -> Result<TargetKind, LifecycleError> {
    match value {
        "installation_id" => Ok(TargetKind::InstallationId),
        "session_id" => Ok(TargetKind::SessionId),
        "replay_id" => Ok(TargetKind::ReplayId),
        _ => Err(LifecycleError::State("target kind is invalid".to_owned())),
    }
}

fn same_request(left: &Request, right: &Request) -> bool {
    left.organization_id == right.organization_id
        && left.project_id == right.project_id
        && left.environment_id == right.environment_id
        && left.kind == right.kind
        && left.target_kind == right.target_kind
        && left.requested_by == right.requested_by
        && left.reason_code == right.reason_code
        && left.idempotency_key == right.idempotency_key
        && left.cutoff_unix_nano == right.cutoff_unix_nano
        && (left.kind != Kind::DataSubject || left.target_digest() == right.target_sha256)
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

async fn suspend_scope(
    transaction: &mut Transaction<'_, Postgres>,
    request: &Request,
) -> Result<(), LifecycleError> {
    if request.kind == Kind::Tenant {
        sqlx::query("UPDATE control.sdk_keys SET status='revoked',revoked_at=coalesce(revoked_at,clock_timestamp()) WHERE organization_id=$1::uuid AND status<>'revoked'")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.data_sources SET status='suspended',updated_at=clock_timestamp() WHERE organization_id=$1::uuid AND status<>'deleted'")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.environments SET status='suspended',updated_at=clock_timestamp() WHERE organization_id=$1::uuid AND status<>'deleted'")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.projects SET status='suspended',updated_at=clock_timestamp() WHERE organization_id=$1::uuid AND status<>'deleted'")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.organizations SET status='suspended',updated_at=clock_timestamp() WHERE id=$1::uuid AND status<>'deleted'")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
    } else {
        sqlx::query("UPDATE control.sdk_keys SET status='revoked',revoked_at=coalesce(revoked_at,clock_timestamp()) WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND status<>'revoked'")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.data_sources SET status='suspended',updated_at=clock_timestamp() WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND status<>'deleted'")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.environments SET status='suspended',updated_at=clock_timestamp() WHERE organization_id=$1::uuid AND project_id=$2::uuid AND id=$3::uuid AND status<>'deleted'")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
    }
    Ok(())
}

async fn matching_canonical_ids(
    transaction: &mut Transaction<'_, Postgres>,
    request: &Request,
) -> Result<Vec<i64>, LifecycleError> {
    let ids = match request.kind {
        Kind::Tenant => sqlx::query_scalar(
            "SELECT id FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid",
        )
        .bind(&request.organization_id)
        .fetch_all(&mut **transaction)
        .await?,
        Kind::Environment => sqlx::query_scalar(
            "SELECT id FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid",
        )
        .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
        .fetch_all(&mut **transaction).await?,
        Kind::Retention => sqlx::query_scalar(
            "SELECT id FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND effective_occurred_at_unix_nano<$4::numeric",
        )
        .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
        .bind(required_cutoff(request)?.to_string()).fetch_all(&mut **transaction).await?,
        Kind::ReplayExpiry => sqlx::query_scalar(
            "SELECT id FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND envelope_kind='replay' AND effective_occurred_at_unix_nano<$4::numeric",
        )
        .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
        .bind(required_cutoff(request)?.to_string()).fetch_all(&mut **transaction).await?,
        Kind::DataSubject => match request.target_kind {
            Some(TargetKind::InstallationId) => sqlx::query_scalar(
                "SELECT id FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND installation_id=$4",
            ).bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
                .bind(&request.target_value).fetch_all(&mut **transaction).await?,
            Some(TargetKind::SessionId) => sqlx::query_scalar(
                "SELECT id FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND session_id=$4",
            ).bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
                .bind(&request.target_value).fetch_all(&mut **transaction).await?,
            Some(TargetKind::ReplayId) => sqlx::query_scalar(
                "SELECT id FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND replay_id=$4",
            ).bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
                .bind(&request.target_value).fetch_all(&mut **transaction).await?,
            None => return Err(LifecycleError::State("subject target kind missing".to_owned())),
        },
    };
    Ok(ids)
}

async fn purge_unreferenced_inbox(
    transaction: &mut Transaction<'_, Postgres>,
    request: &Request,
) -> Result<i64, LifecycleError> {
    if request.kind == Kind::DataSubject {
        return Ok(0);
    }
    let cutoff = if matches!(request.kind, Kind::Retention | Kind::ReplayExpiry) {
        Some(
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(required_cutoff(request)?))
                .map_err(|_| {
                    LifecycleError::State("retention cutoff timestamp is invalid".to_owned())
                })?,
        )
    } else {
        None
    };
    let bytes: i64 =
        match request.kind {
            Kind::Tenant => sqlx::query_scalar(
                r"SELECT coalesce(sum(octet_length(payload)),0)::bigint FROM ingest.inbox AS inbox
              WHERE organization_id=$1::uuid AND NOT EXISTS
              (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)",
            )
            .bind(&request.organization_id)
            .fetch_one(&mut **transaction)
            .await?,
            Kind::Environment => sqlx::query_scalar(
                r"SELECT coalesce(sum(octet_length(payload)),0)::bigint FROM ingest.inbox AS inbox
              WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid
              AND NOT EXISTS (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)",
            )
            .bind(&request.organization_id)
            .bind(&request.project_id)
            .bind(&request.environment_id)
            .fetch_one(&mut **transaction)
            .await?,
            Kind::Retention => sqlx::query_scalar(
                r"SELECT coalesce(sum(octet_length(payload)),0)::bigint FROM ingest.inbox AS inbox
              WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid
              AND server_received_at<$4 AND NOT EXISTS
              (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)",
            )
            .bind(&request.organization_id)
            .bind(&request.project_id)
            .bind(&request.environment_id)
            .bind(cutoff)
            .fetch_one(&mut **transaction)
            .await?,
            Kind::ReplayExpiry => sqlx::query_scalar(
                r"SELECT coalesce(sum(octet_length(payload)),0)::bigint FROM ingest.inbox AS inbox
              WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid
              AND server_received_at<$4 AND signal_kind='replay' AND NOT EXISTS
              (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)",
            )
            .bind(&request.organization_id)
            .bind(&request.project_id)
            .bind(&request.environment_id)
            .bind(cutoff)
            .fetch_one(&mut **transaction)
            .await?,
            Kind::DataSubject => 0,
        };
    match request.kind {
        Kind::Tenant => {
            sqlx::query("DELETE FROM ingest.inbox AS inbox WHERE organization_id=$1::uuid AND NOT EXISTS (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        }
        Kind::Environment => {
            sqlx::query("DELETE FROM ingest.inbox AS inbox WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND NOT EXISTS (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
        }
        Kind::Retention => {
            sqlx::query("DELETE FROM ingest.inbox AS inbox WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND server_received_at<$4 AND NOT EXISTS (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).bind(cutoff).execute(&mut **transaction).await?;
        }
        Kind::ReplayExpiry => {
            sqlx::query("DELETE FROM ingest.inbox AS inbox WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND server_received_at<$4 AND signal_kind='replay' AND NOT EXISTS (SELECT 1 FROM ingest.canonical_envelopes WHERE inbox_id=inbox.id)")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).bind(cutoff).execute(&mut **transaction).await?;
        }
        Kind::DataSubject => {}
    }
    Ok(bytes)
}

async fn purge_operational_buckets(
    transaction: &mut Transaction<'_, Postgres>,
    request: &Request,
) -> Result<(), LifecycleError> {
    match request.kind {
        Kind::DataSubject => {}
        Kind::Tenant => {
            sqlx::query("DELETE FROM ingest.rate_buckets WHERE organization_id=$1::uuid")
                .bind(&request.organization_id)
                .execute(&mut **transaction)
                .await?;
            sqlx::query("DELETE FROM ingest.daily_buckets WHERE organization_id=$1::uuid")
                .bind(&request.organization_id)
                .execute(&mut **transaction)
                .await?;
        }
        Kind::Environment => {
            sqlx::query("DELETE FROM ingest.rate_buckets WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid")
                .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
            sqlx::query("DELETE FROM ingest.daily_buckets WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid")
                .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
        }
        Kind::Retention => {
            let cutoff =
                OffsetDateTime::from_unix_timestamp_nanos(i128::from(required_cutoff(request)?))
                    .map_err(|_| LifecycleError::State("retention cutoff invalid".to_owned()))?;
            sqlx::query("DELETE FROM ingest.rate_buckets WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND minute_start<$4")
                .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).bind(cutoff).execute(&mut **transaction).await?;
            sqlx::query("DELETE FROM ingest.daily_buckets WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND day_start<$4::date")
                .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).bind(cutoff).execute(&mut **transaction).await?;
        }
        Kind::ReplayExpiry => {
            let cutoff =
                OffsetDateTime::from_unix_timestamp_nanos(i128::from(required_cutoff(request)?))
                    .map_err(|_| LifecycleError::State("retention cutoff invalid".to_owned()))?;
            sqlx::query("DELETE FROM ingest.daily_buckets WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND day_start<$4::date")
                .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).bind(cutoff).execute(&mut **transaction).await?;
        }
    }
    Ok(())
}

async fn invalidate_derived(
    transaction: &mut Transaction<'_, Postgres>,
    request: &Request,
) -> Result<(), LifecycleError> {
    let environments: Vec<(String, String)> = if request.kind == Kind::Tenant {
        sqlx::query_as("SELECT project_id::text,id::text FROM control.environments WHERE organization_id=$1::uuid ORDER BY project_id,id")
            .bind(&request.organization_id).fetch_all(&mut **transaction).await?
    } else {
        sqlx::query_as("SELECT project_id::text,id::text FROM control.environments WHERE organization_id=$1::uuid AND project_id=$2::uuid AND id=$3::uuid")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
            .fetch_all(&mut **transaction).await?
    };
    for (project, environment) in environments {
        let generation: i64 = sqlx::query_scalar(
            r"INSERT INTO lifecycle.environment_generations (organization_id,project_id,
              environment_id,generation,reason_code,request_id) VALUES ($1::uuid,$2::uuid,$3::uuid,1,$4,$5::uuid)
              ON CONFLICT (environment_id) DO UPDATE SET generation=lifecycle.environment_generations.generation+1,
              reason_code=EXCLUDED.reason_code,request_id=EXCLUDED.request_id,invalidated_at=clock_timestamp()
              RETURNING generation",
        ).bind(&request.organization_id).bind(&project).bind(&environment).bind(&request.reason_code)
            .bind(&request.id).fetch_one(&mut **transaction).await?;
        for kind in [
            "query",
            "trace_index",
            "replay_index",
            "funnel",
            "cohort",
            "aggregate",
        ] {
            sqlx::query("INSERT INTO lifecycle.derived_rebuilds (request_id,organization_id,project_id,environment_id,dataset_kind,generation) VALUES ($1::uuid,$2::uuid,$3::uuid,$4::uuid,$5,$6) ON CONFLICT (request_id,environment_id,dataset_kind) DO NOTHING")
                .bind(&request.id).bind(&request.organization_id).bind(&project).bind(&environment).bind(kind).bind(generation)
                .execute(&mut **transaction).await?;
        }
    }
    Ok(())
}

async fn deactivate_scope(
    transaction: &mut Transaction<'_, Postgres>,
    request: &Request,
) -> Result<(), LifecycleError> {
    if request.kind == Kind::Environment {
        sqlx::query("UPDATE control.sdk_keys SET status='revoked',revoked_at=coalesce(revoked_at,clock_timestamp()) WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND status<>'revoked'")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.data_sources SET status='deleted',updated_at=clock_timestamp() WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.environments SET status='deleted',updated_at=clock_timestamp() WHERE organization_id=$1::uuid AND project_id=$2::uuid AND id=$3::uuid")
            .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id).execute(&mut **transaction).await?;
    } else if request.kind == Kind::Tenant {
        sqlx::query("UPDATE control.sdk_keys SET status='revoked',revoked_at=coalesce(revoked_at,clock_timestamp()) WHERE organization_id=$1::uuid AND status<>'revoked'")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.data_sources SET status='deleted',updated_at=clock_timestamp() WHERE organization_id=$1::uuid")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.environments SET status='deleted',updated_at=clock_timestamp() WHERE organization_id=$1::uuid")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.projects SET status='deleted',updated_at=clock_timestamp() WHERE organization_id=$1::uuid")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.organization_memberships SET status='suspended',updated_at=clock_timestamp() WHERE organization_id=$1::uuid")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
        sqlx::query("UPDATE control.organizations SET status='deleted',deleted_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=$1::uuid")
            .bind(&request.organization_id).execute(&mut **transaction).await?;
    }
    Ok(())
}

fn required_cutoff(request: &Request) -> Result<u64, LifecycleError> {
    request
        .cutoff_unix_nano
        .ok_or_else(|| LifecycleError::State("retention cutoff missing".to_owned()))
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
