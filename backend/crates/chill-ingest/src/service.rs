use std::{sync::Arc, time::Duration};

use chill_control_plane::{AuthenticatedSDKKey, ControlPlaneError, Store};
use serde_json::json;
use sqlx::{Postgres, Transaction, types::Json};
use time::OffsetDateTime;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    AdmissionError, Candidate, ErrorCode, Limits, Receipt, SignalKind, ValidatedRequest,
    validate_candidate,
};

/// Authenticated durable admission service with bounded process concurrency.
#[derive(Clone)]
pub struct Service {
    control: Store,
    limits: Limits,
    slots: Arc<Semaphore>,
}

/// Process-level admission held while untrusted request work is performed.
#[derive(Clone)]
pub(crate) struct AdmissionPermit {
    _permit: Arc<OwnedSemaphorePermit>,
}

impl Service {
    /// Creates an admission service after validating its process limits.
    ///
    /// # Errors
    ///
    /// Returns invalid input for limits outside the production envelope.
    pub fn new(control: Store, limits: Limits) -> Result<Self, AdmissionError> {
        limits.validate()?;
        Ok(Self {
            control,
            limits,
            slots: Arc::new(Semaphore::new(limits.maximum_concurrent)),
        })
    }

    /// Validates, authenticates, quota-checks, and durably appends one request.
    ///
    /// # Errors
    ///
    /// Returns stable admission errors for validation, authorization, quota,
    /// backpressure, idempotency conflict, or transient dependency failure.
    pub async fn accept(&self, candidate: Candidate) -> Result<Receipt, AdmissionError> {
        let permit = self.try_admit()?;
        self.accept_admitted(candidate, permit).await
    }

    pub(crate) fn limits(&self) -> Limits {
        self.limits
    }

    pub(crate) fn try_admit(&self) -> Result<AdmissionPermit, AdmissionError> {
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            AdmissionError::retry(
                ErrorCode::Backpressure,
                "ingestion concurrency is saturated",
                Duration::from_secs(1),
            )
        })?;
        Ok(AdmissionPermit {
            _permit: Arc::new(permit),
        })
    }

    pub(crate) async fn accept_admitted(
        &self,
        candidate: Candidate,
        _permit: AdmissionPermit,
    ) -> Result<Receipt, AdmissionError> {
        let credential = candidate.credential.clone();
        let request = validate_candidate(candidate, self.limits)?;
        let key = self
            .control
            .authenticate_sdk_key(&credential)
            .await
            .map_err(authentication_error)?;
        let required_scope = if request.kind == SignalKind::Replay {
            "ingest:replay"
        } else {
            "ingest:otlp"
        };
        if !key.scopes.iter().any(|scope| scope == required_scope) {
            return Err(AdmissionError::new(
                ErrorCode::Forbidden,
                "SDK key lacks the required ingestion scope",
            ));
        }
        let mut transaction = self
            .control
            .begin_tenant(&key.organization_id)
            .await
            .map_err(|error| {
                AdmissionError::with_source(
                    ErrorCode::Unavailable,
                    "durable ingestion is temporarily unavailable",
                    error,
                )
            })?;
        let receipt = append(&mut transaction, &key, &request).await?;
        transaction.commit().await.map_err(database_unavailable)?;
        Ok(receipt)
    }
}

fn authentication_error(error: ControlPlaneError) -> AdmissionError {
    if matches!(error, ControlPlaneError::Unauthorized) {
        AdmissionError::new(ErrorCode::Unauthorized, "SDK key is invalid")
    } else {
        AdmissionError::with_source(
            ErrorCode::Unavailable,
            "SDK key authentication is temporarily unavailable",
            error,
        )
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "idempotency, schema checks, quota reservation, and inbox persistence are one transaction"
)]
async fn append(
    transaction: &mut Transaction<'_, Postgres>,
    key: &AuthenticatedSDKKey,
    request: &ValidatedRequest,
) -> Result<Receipt, AdmissionError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1229472069))")
        .bind(format!(
            "{}|{}|{}",
            key.environment_id,
            request.kind.as_str(),
            request.idempotency_key
        ))
        .execute(&mut **transaction)
        .await
        .map_err(database_unavailable)?;
    let existing = sqlx::query_as::<_, (i64, Vec<u8>, OffsetDateTime)>(
        r"
        SELECT id, payload_sha256, server_received_at FROM ingest.inbox
        WHERE environment_id = $1::uuid AND signal_kind = $2 AND request_id = $3
    ",
    )
    .bind(&key.environment_id)
    .bind(request.kind.as_str())
    .bind(&request.idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_unavailable)?;
    if let Some(row) = existing {
        if row.1.as_slice() != request.payload_digest.as_slice() {
            return Err(AdmissionError::new(
                ErrorCode::Conflict,
                "idempotency key was already used for different content",
            ));
        }
        return Ok(Receipt {
            inbox_id: row.0,
            idempotency_key: request.idempotency_key.clone(),
            duplicate: true,
            server_received_at: row.2,
        });
    }
    for schema in &request.schema_refs {
        let exists = sqlx::query_scalar::<_, bool>(
            r"
            SELECT EXISTS (
                SELECT 1 FROM control.behavior_schemas
                WHERE organization_id = $1::uuid AND project_id = $2::uuid
                  AND version = $3 AND schema_url = $4 AND status = 'active')
        ",
        )
        .bind(&key.organization_id)
        .bind(&key.project_id)
        .bind(&schema.version)
        .bind(&schema.url)
        .fetch_one(&mut **transaction)
        .await
        .map_err(database_unavailable)?;
        if !exists {
            return Err(AdmissionError::new(
                ErrorCode::Invalid,
                format!(
                    "request references inactive or unknown Chill schema {} at version {}",
                    schema.url, schema.version
                ),
            ));
        }
    }
    let quota = sqlx::query_as::<_, (i64, i64, i64)>(
        r"
        SELECT requests_per_minute::bigint, records_per_minute::bigint,
            replay_bytes_per_day FROM control.quotas
        WHERE organization_id = $1::uuid AND project_id = $2::uuid
          AND environment_id = $3::uuid
    ",
    )
    .bind(&key.organization_id)
    .bind(&key.project_id)
    .bind(&key.environment_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_unavailable)?
    .ok_or_else(|| {
        AdmissionError::new(
            ErrorCode::Forbidden,
            "environment has no active ingestion quota",
        )
    })?;
    let record_count = i64::try_from(request.record_count).map_err(|_| {
        AdmissionError::new(ErrorCode::Invalid, "record count exceeds database range")
    })?;
    let replay_bytes = if request.kind == SignalKind::Replay {
        i64::try_from(request.payload.len()).map_err(|_| {
            AdmissionError::new(ErrorCode::TooLarge, "replay payload exceeds database range")
        })?
    } else {
        0
    };
    if record_count > quota.1 || replay_bytes > quota.2 {
        return Err(quota_error());
    }
    let accepted = sqlx::query_scalar::<_, i64>(
        r"
        INSERT INTO ingest.rate_buckets (
            organization_id, project_id, environment_id, minute_start,
            request_count, record_count, replay_bytes
        ) VALUES ($1::uuid, $2::uuid, $3::uuid, date_trunc('minute', clock_timestamp()),
            1, $4, $5)
        ON CONFLICT (environment_id, minute_start) DO UPDATE
        SET request_count = ingest.rate_buckets.request_count + 1,
            record_count = ingest.rate_buckets.record_count + EXCLUDED.record_count,
            replay_bytes = ingest.rate_buckets.replay_bytes + EXCLUDED.replay_bytes,
            updated_at = clock_timestamp()
        WHERE ingest.rate_buckets.request_count + 1 <= $6
          AND ingest.rate_buckets.record_count + EXCLUDED.record_count <= $7
        RETURNING request_count::bigint
    ",
    )
    .bind(&key.organization_id)
    .bind(&key.project_id)
    .bind(&key.environment_id)
    .bind(record_count)
    .bind(replay_bytes)
    .bind(quota.0)
    .bind(quota.1)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_unavailable)?;
    if accepted.is_none() {
        return Err(quota_error());
    }
    if replay_bytes > 0 {
        let accepted = sqlx::query_scalar::<_, i64>(
            r"
            INSERT INTO ingest.daily_buckets (
                organization_id, project_id, environment_id, day_start, replay_bytes
            ) VALUES ($1::uuid, $2::uuid, $3::uuid,
                (clock_timestamp() AT TIME ZONE 'UTC')::date, $4)
            ON CONFLICT (environment_id, day_start) DO UPDATE
            SET replay_bytes = ingest.daily_buckets.replay_bytes + EXCLUDED.replay_bytes,
                updated_at = clock_timestamp()
            WHERE ingest.daily_buckets.replay_bytes + EXCLUDED.replay_bytes <= $5
            RETURNING replay_bytes
        ",
        )
        .bind(&key.organization_id)
        .bind(&key.project_id)
        .bind(&key.environment_id)
        .bind(replay_bytes)
        .bind(quota.2)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_unavailable)?;
        if accepted.is_none() {
            return Err(quota_error());
        }
    }
    let metadata = json!({
        "schema_refs": request.schema_refs,
        "replay": request.replay,
    });
    let row = sqlx::query_as::<_, (i64, OffsetDateTime)>(
        r"
        INSERT INTO ingest.inbox (
            organization_id, project_id, environment_id, data_source_id,
            sdk_key_id, request_id, signal_kind, payload_format, payload,
            payload_sha256, record_count, metadata
        ) VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5::uuid, $6,
            $7, $8, $9, $10, $11, $12) RETURNING id, server_received_at
    ",
    )
    .bind(&key.organization_id)
    .bind(&key.project_id)
    .bind(&key.environment_id)
    .bind(&key.data_source_id)
    .bind(&key.key_id)
    .bind(&request.idempotency_key)
    .bind(request.kind.as_str())
    .bind(request.format.as_str())
    .bind(&request.payload)
    .bind(request.payload_digest.as_slice())
    .bind(record_count)
    .bind(Json(metadata))
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_unavailable)?;
    Ok(Receipt {
        inbox_id: row.0,
        idempotency_key: request.idempotency_key.clone(),
        duplicate: false,
        server_received_at: row.1,
    })
}

fn database_unavailable(error: sqlx::Error) -> AdmissionError {
    AdmissionError::with_source(
        ErrorCode::Unavailable,
        "durable ingestion is temporarily unavailable",
        error,
    )
}

fn quota_error() -> AdmissionError {
    AdmissionError::retry(
        ErrorCode::Quota,
        "environment ingestion quota is exhausted",
        Duration::from_mins(1),
    )
}
