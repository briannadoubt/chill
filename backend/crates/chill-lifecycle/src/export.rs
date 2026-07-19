use std::{
    io::{Cursor, Write as _},
    sync::LazyLock,
};

use chill_control_plane::{ControlPlaneError, Store};
use chill_lake::{Row, decode_parquet};
use chill_objects::{ImmutableStore, ObjectError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{Postgres, Transaction, types::Json};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

static REASON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$")
        .unwrap_or_else(|error| unreachable!("static export reason regex: {error}"))
});

/// Privacy export scope.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    /// All organization data, policies, and audit history.
    Tenant,
    /// Records matching one selected pseudonymous identifier.
    DataSubject,
}

impl ExportKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tenant => "tenant",
            Self::DataSubject => "data_subject",
        }
    }
}

/// Selected pseudonymous identifier for a subject export.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportTargetKind {
    /// Installation identifier.
    InstallationId,
    /// Session identifier.
    SessionId,
    /// Replay identifier.
    ReplayId,
}

impl ExportTargetKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::InstallationId => "installation_id",
            Self::SessionId => "session_id",
            Self::ReplayId => "replay_id",
        }
    }
}

/// One validated privacy-export request.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExportRequest {
    /// Organization UUID.
    pub organization_id: String,
    /// Project UUID for a subject export.
    #[serde(default)]
    pub project_id: String,
    /// Environment UUID for a subject export.
    #[serde(default)]
    pub environment_id: String,
    /// Export scope.
    pub kind: ExportKind,
    /// Subject identifier kind.
    pub target_kind: Option<ExportTargetKind>,
    /// Subject plaintext, never persisted or included in the manifest.
    #[serde(skip)]
    pub target_value: String,
    /// Stable requester identity.
    pub requested_by: String,
    /// Auditable machine-readable reason.
    pub reason_code: String,
    /// Tenant-scoped idempotency key.
    pub idempotency_key: String,
}

impl ExportRequest {
    /// Validates scope and bounded audit fields.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed UUIDs, ambiguous scope, or unbounded fields.
    pub fn validate(&self) -> Result<(), ExportError> {
        canonical_uuid(&self.organization_id)?;
        match self.kind {
            ExportKind::Tenant => {
                if !self.project_id.is_empty()
                    || !self.environment_id.is_empty()
                    || self.target_kind.is_some()
                    || !self.target_value.is_empty()
                {
                    return Err(ExportError::InvalidRequest(
                        "tenant export contains subject scope",
                    ));
                }
            }
            ExportKind::DataSubject => {
                canonical_uuid(&self.project_id)?;
                canonical_uuid(&self.environment_id)?;
                if self.target_kind.is_none()
                    || self.target_value.is_empty()
                    || self.target_value.len() > 256
                    || self.target_value.trim() != self.target_value
                {
                    return Err(ExportError::InvalidRequest(
                        "subject export target is invalid",
                    ));
                }
            }
        }
        if self.requested_by.is_empty()
            || self.requested_by.len() > 160
            || !REASON.is_match(&self.reason_code)
            || self.idempotency_key.is_empty()
            || self.idempotency_key.len() > 128
        {
            return Err(ExportError::InvalidRequest(
                "audit or idempotency fields are invalid",
            ));
        }
        Ok(())
    }

    fn target_digest(&self) -> Option<[u8; 32]> {
        (self.kind == ExportKind::DataSubject)
            .then(|| Sha256::digest(self.target_value.as_bytes()).into())
    }

    fn matches(&self, row: &Row) -> bool {
        if self.kind == ExportKind::Tenant {
            return true;
        }
        match self.target_kind {
            Some(ExportTargetKind::InstallationId) => row.installation_id.as_deref(),
            Some(ExportTargetKind::SessionId) => row.session_id.as_deref(),
            Some(ExportTargetKind::ReplayId) => row.replay_id.as_deref(),
            None => None,
        }
        .is_some_and(|value| value == self.target_value)
    }
}

/// Hard privacy-export resource bounds.
#[derive(Clone, Debug)]
pub struct ExportConfiguration {
    /// Maximum committed source files.
    pub maximum_files: usize,
    /// Maximum aggregate compressed source bytes.
    pub maximum_source_bytes: i64,
    /// Maximum selected records.
    pub maximum_records: usize,
    /// Maximum final archive bytes.
    pub maximum_artifact_bytes: usize,
    /// Maximum tenant audit records.
    pub maximum_audit_records: usize,
}

impl Default for ExportConfiguration {
    fn default() -> Self {
        Self {
            maximum_files: 10_000,
            maximum_source_bytes: 10 << 30,
            maximum_records: 1_000_000,
            maximum_artifact_bytes: 1 << 30,
            maximum_audit_records: 100_000,
        }
    }
}

/// Delivery receipt recorded only after the callback succeeds.
#[derive(Clone, Debug, Serialize)]
pub struct ExportReceipt {
    /// Export-request UUID.
    pub request_id: String,
    /// Lowercase artifact SHA-256.
    pub artifact_sha256: String,
    /// Selected canonical record count.
    pub record_count: usize,
    /// ZIP byte count.
    pub artifact_bytes: usize,
    /// Source Parquet file count.
    pub source_file_count: usize,
    /// Durable request creation time.
    pub created_at: OffsetDateTime,
}

/// Private export artifact passed across the operator-owned delivery boundary.
#[derive(Clone, Debug)]
pub struct ExportArtifact {
    /// Artifact receipt.
    pub receipt: ExportReceipt,
    /// ZIP body; the callback must not retain this beyond its approved destination.
    pub body: Vec<u8>,
}

/// Privacy-export failure.
#[derive(Debug, Error)]
pub enum ExportError {
    /// Public request validation failed.
    #[error("privacy export request is invalid: {0}")]
    InvalidRequest(&'static str),
    /// Dependencies or resource bounds are unsafe.
    #[error("privacy export configuration is invalid")]
    InvalidConfiguration,
    /// The idempotency key already identifies a request.
    #[error("privacy export idempotency key is already in use")]
    IdempotencyConflict,
    /// A completed request cannot be delivered again implicitly.
    #[error("privacy export idempotency key already completed")]
    AlreadyCompleted,
    /// A configured bound was exceeded.
    #[error("privacy export exceeds its {0} limit")]
    Limit(&'static str),
    /// A source or durable state violated an invariant.
    #[error("privacy export state is invalid: {0}")]
    State(String),
    /// Delivery failed; durable request is marked failed.
    #[error("deliver privacy export: {0}")]
    Delivery(String),
    /// Tenant transaction setup failed.
    #[error("privacy export tenant operation failed: {0}")]
    Control(#[from] ControlPlaneError),
    /// Database operation failed.
    #[error("privacy export database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// Object operation failed.
    #[error("privacy export object operation failed: {0}")]
    Object(#[from] ObjectError),
    /// Parquet processing failed.
    #[error("privacy export lake operation failed: {0}")]
    Lake(#[from] chill_lake::LakeError),
    /// JSON encoding failed.
    #[error("privacy export JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    /// ZIP encoding failed.
    #[error("privacy export ZIP encoding failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// Archive writing failed.
    #[error("privacy export archive write failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
struct SourceFile {
    key: String,
    digest: [u8; 32],
    byte_count: i64,
}

type RequestRow = (
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    Option<Vec<u8>>,
    String,
    String,
    String,
    OffsetDateTime,
);
type PolicyTuple = (
    String,
    String,
    i64,
    String,
    bool,
    Vec<String>,
    String,
    String,
);
type AuditTuple = (
    i64,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Json<Value>,
    String,
);

#[derive(Debug, Serialize)]
struct PolicyRecord {
    project_id: String,
    environment_id: String,
    revision: i64,
    policy_version: String,
    enabled: bool,
    disabled_capture_classes: Vec<String>,
    status: String,
    effective_at: String,
}

#[derive(Debug, Serialize)]
struct AuditRecord {
    id: i64,
    actor_user_id: Option<String>,
    action: String,
    target_type: String,
    target_id: Option<String>,
    request_id: Option<String>,
    reason_code: Option<String>,
    details: Value,
    occurred_at: String,
}

/// Bounded, tenant-safe privacy export builder.
#[derive(Clone)]
pub struct Exporter {
    control: Store,
    objects: ImmutableStore,
    configuration: ExportConfiguration,
}

impl Exporter {
    /// Creates an exporter over the shared control-plane and object boundaries.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or unreasonably small bounds.
    pub fn new(
        control: Store,
        objects: ImmutableStore,
        configuration: ExportConfiguration,
    ) -> Result<Self, ExportError> {
        if configuration.maximum_files == 0
            || configuration.maximum_source_bytes < 1
            || configuration.maximum_records == 0
            || configuration.maximum_artifact_bytes < 1 << 20
            || configuration.maximum_audit_records == 0
        {
            return Err(ExportError::InvalidConfiguration);
        }
        Ok(Self {
            control,
            objects,
            configuration,
        })
    }

    /// Builds and delivers an export, recording completion only after delivery succeeds.
    ///
    /// # Errors
    ///
    /// Returns a validation, catalog, integrity, limit, archive, delivery, or durable-state error.
    pub async fn run<F, E>(
        &self,
        request: ExportRequest,
        deliver: F,
    ) -> Result<ExportReceipt, ExportError>
    where
        F: FnOnce(ExportArtifact) -> Result<(), E>,
        E: std::fmt::Display,
    {
        request.validate()?;
        let receipt = self.create_request(&request).await?;
        let request_id = receipt.request_id.clone();
        let result = self.build_and_deliver(&request, receipt, deliver).await;
        if result.is_err() {
            let _ = self.fail(&request.organization_id, &request_id).await;
        }
        result
    }

    async fn build_and_deliver<F, E>(
        &self,
        request: &ExportRequest,
        mut receipt: ExportReceipt,
        deliver: F,
    ) -> Result<ExportReceipt, ExportError>
    where
        F: FnOnce(ExportArtifact) -> Result<(), E>,
        E: std::fmt::Display,
    {
        let (files, policies, audits) = self.catalog(request).await?;
        let mut rows = Vec::new();
        let mut source_bytes = 0_i64;
        for file in &files {
            source_bytes = source_bytes
                .checked_add(file.byte_count)
                .ok_or(ExportError::Limit("source bytes"))?;
            if source_bytes > self.configuration.maximum_source_bytes {
                return Err(ExportError::Limit("source bytes"));
            }
            let body = self.objects.get(&file.key).await?;
            if i64::try_from(body.len()).ok() != Some(file.byte_count)
                || Sha256::digest(&body).as_slice() != file.digest
            {
                return Err(ExportError::State(format!(
                    "source {} metadata changed",
                    file.key
                )));
            }
            for row in decode_parquet(body)? {
                if request.matches(&row) {
                    rows.push(row);
                    if rows.len() > self.configuration.maximum_records {
                        return Err(ExportError::Limit("records"));
                    }
                }
            }
        }
        rows.sort_by_key(|row| {
            (
                row.effective_occurred_at_unix_nano,
                row.canonical_envelope_id,
            )
        });
        let body = build_archive(&receipt, request, &rows, &policies, &audits)?;
        if body.len() > self.configuration.maximum_artifact_bytes {
            return Err(ExportError::Limit("artifact bytes"));
        }
        receipt.artifact_sha256 = hex::encode(Sha256::digest(&body));
        receipt.record_count = rows.len();
        receipt.artifact_bytes = body.len();
        receipt.source_file_count = files.len();
        deliver(ExportArtifact {
            receipt: receipt.clone(),
            body,
        })
        .map_err(|error| ExportError::Delivery(error.to_string()))?;
        self.complete(&request.organization_id, &receipt).await?;
        Ok(receipt)
    }

    async fn create_request(&self, request: &ExportRequest) -> Result<ExportReceipt, ExportError> {
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        let digest = request.target_digest();
        let target_kind = request.target_kind.map(ExportTargetKind::as_str);
        let created = sqlx::query(
            r"INSERT INTO compliance.export_requests
            (organization_id,project_id,environment_id,kind,target_kind,target_sha256,
             requested_by,reason_code,idempotency_key)
            VALUES ($1::uuid,$2::uuid,$3::uuid,$4,$5,$6,$7,$8,$9)
            ON CONFLICT (organization_id,idempotency_key) DO NOTHING",
        )
        .bind(&request.organization_id)
        .bind(nonempty(&request.project_id))
        .bind(nonempty(&request.environment_id))
        .bind(request.kind.as_str())
        .bind(target_kind)
        .bind(digest.as_ref().map(<[u8; 32]>::as_slice))
        .bind(&request.requested_by)
        .bind(&request.reason_code)
        .bind(&request.idempotency_key)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        let row: RequestRow = sqlx::query_as(
            r"SELECT id::text,
            project_id::text,environment_id::text,kind,target_kind,target_sha256,
            requested_by,reason_code,status,created_at FROM compliance.export_requests
            WHERE organization_id=$1::uuid AND idempotency_key=$2",
        )
        .bind(&request.organization_id)
        .bind(&request.idempotency_key)
        .fetch_one(&mut *transaction)
        .await?;
        if !created {
            if !same_request(request, &row, digest.as_ref()) {
                return Err(ExportError::IdempotencyConflict);
            }
            return Err(if row.8 == "completed" {
                ExportError::AlreadyCompleted
            } else {
                ExportError::IdempotencyConflict
            });
        }
        sqlx::query(
            r"INSERT INTO control.audit_log
            (organization_id,action,target_type,target_id,request_id,reason_code,details)
            VALUES ($1::uuid,'privacy.export.requested','export_request',$2::uuid,$3,$4,
            jsonb_build_object('kind',$5::text,'target_kind',$6::text))",
        )
        .bind(&request.organization_id)
        .bind(&row.0)
        .bind(&request.idempotency_key)
        .bind(&request.reason_code)
        .bind(request.kind.as_str())
        .bind(target_kind)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ExportReceipt {
            request_id: row.0,
            artifact_sha256: String::new(),
            record_count: 0,
            artifact_bytes: 0,
            source_file_count: 0,
            created_at: row.9,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the catalog snapshot keeps tenant-scoped files, policies, and audit ordering together"
    )]
    async fn catalog(
        &self,
        request: &ExportRequest,
    ) -> Result<(Vec<SourceFile>, Vec<PolicyRecord>, Vec<AuditRecord>), ExportError> {
        let mut transaction = self.control.begin_tenant(&request.organization_id).await?;
        ensure_no_uncommitted_matching_records(&mut transaction, request).await?;
        let limit = i64::try_from(self.configuration.maximum_files)
            .map_err(|_| ExportError::InvalidConfiguration)?
            .saturating_add(1);
        let raw_files: Vec<(String, Vec<u8>, i64)> = if request.kind == ExportKind::Tenant {
            sqlx::query_as(
                r"SELECT object_key,object_sha256,byte_count FROM lake.export_batches
                WHERE organization_id=$1::uuid AND status='committed'
                ORDER BY partition_day,partition_hour,envelope_kind,batch_id LIMIT $2",
            )
            .bind(&request.organization_id)
            .bind(limit)
            .fetch_all(&mut *transaction)
            .await?
        } else {
            sqlx::query_as(
                r"SELECT object_key,object_sha256,byte_count FROM lake.export_batches
                WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid
                  AND status='committed'
                ORDER BY partition_day,partition_hour,envelope_kind,batch_id LIMIT $4",
            )
            .bind(&request.organization_id)
            .bind(&request.project_id)
            .bind(&request.environment_id)
            .bind(limit)
            .fetch_all(&mut *transaction)
            .await?
        };
        if raw_files.len() > self.configuration.maximum_files {
            return Err(ExportError::Limit("source files"));
        }
        let mut files = Vec::with_capacity(raw_files.len());
        for (key, digest, byte_count) in raw_files {
            let digest: [u8; 32] = digest
                .try_into()
                .map_err(|_| ExportError::State("source digest size is invalid".to_owned()))?;
            if byte_count < 1 {
                return Err(ExportError::State(
                    "source byte count is invalid".to_owned(),
                ));
            }
            files.push(SourceFile {
                key,
                digest,
                byte_count,
            });
        }
        let raw_policies: Vec<PolicyTuple> = if request.kind == ExportKind::Tenant {
            sqlx::query_as(r"SELECT project_id::text,environment_id::text,revision,policy_version,
                enabled,disabled_capture_classes,status,effective_at::text FROM control.collection_policies
                WHERE organization_id=$1::uuid ORDER BY project_id,environment_id,revision")
                .bind(&request.organization_id).fetch_all(&mut *transaction).await?
        } else {
            sqlx::query_as(r"SELECT project_id::text,environment_id::text,revision,policy_version,
                enabled,disabled_capture_classes,status,effective_at::text FROM control.collection_policies
                WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid
                ORDER BY project_id,environment_id,revision")
                .bind(&request.organization_id).bind(&request.project_id).bind(&request.environment_id)
                .fetch_all(&mut *transaction).await?
        };
        let policies = raw_policies
            .into_iter()
            .map(|row| PolicyRecord {
                project_id: row.0,
                environment_id: row.1,
                revision: row.2,
                policy_version: row.3,
                enabled: row.4,
                disabled_capture_classes: row.5,
                status: row.6,
                effective_at: row.7,
            })
            .collect();
        let audits = if request.kind == ExportKind::Tenant {
            let audit_limit = i64::try_from(self.configuration.maximum_audit_records)
                .map_err(|_| ExportError::InvalidConfiguration)?
                .saturating_add(1);
            let rows: Vec<AuditTuple> = sqlx::query_as(
                r"SELECT id,
                actor_user_id::text,action,target_type,target_id::text,request_id,reason_code,
                details,occurred_at::text FROM control.audit_log WHERE organization_id=$1::uuid
                ORDER BY occurred_at,id LIMIT $2",
            )
            .bind(&request.organization_id)
            .bind(audit_limit)
            .fetch_all(&mut *transaction)
            .await?;
            if rows.len() > self.configuration.maximum_audit_records {
                return Err(ExportError::Limit("audit records"));
            }
            rows.into_iter()
                .map(|row| AuditRecord {
                    id: row.0,
                    actor_user_id: row.1,
                    action: row.2,
                    target_type: row.3,
                    target_id: row.4,
                    request_id: row.5,
                    reason_code: row.6,
                    details: row.7.0,
                    occurred_at: row.8,
                })
                .collect()
        } else {
            Vec::new()
        };
        transaction.commit().await?;
        Ok((files, policies, audits))
    }

    async fn complete(
        &self,
        organization: &str,
        receipt: &ExportReceipt,
    ) -> Result<(), ExportError> {
        let digest = hex::decode(&receipt.artifact_sha256)
            .map_err(|_| ExportError::State("artifact digest is invalid".to_owned()))?;
        let mut transaction = self.control.begin_tenant(organization).await?;
        let changed = sqlx::query(
            r"UPDATE compliance.export_requests SET status='completed',
            artifact_sha256=$1,record_count=$2,artifact_bytes=$3,source_file_count=$4,
            completed_at=clock_timestamp() WHERE id=$5::uuid AND status='running'",
        )
        .bind(digest)
        .bind(i64::try_from(receipt.record_count).map_err(|_| ExportError::Limit("records"))?)
        .bind(
            i64::try_from(receipt.artifact_bytes)
                .map_err(|_| ExportError::Limit("artifact bytes"))?,
        )
        .bind(
            i32::try_from(receipt.source_file_count)
                .map_err(|_| ExportError::Limit("source files"))?,
        )
        .bind(&receipt.request_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(ExportError::State(
                "export request was not running".to_owned(),
            ));
        }
        sqlx::query(r"INSERT INTO control.audit_log
            (organization_id,action,target_type,target_id,request_id,reason_code,details)
            VALUES ($1::uuid,'privacy.export.completed','export_request',$2::uuid,$2,
            'privacy.export_completed',jsonb_build_object('artifact_sha256',$3::text,
            'record_count',$4::bigint,'artifact_bytes',$5::bigint,'source_file_count',$6::integer))")
            .bind(organization).bind(&receipt.request_id).bind(&receipt.artifact_sha256)
            .bind(i64::try_from(receipt.record_count).unwrap_or(i64::MAX))
            .bind(i64::try_from(receipt.artifact_bytes).unwrap_or(i64::MAX))
            .bind(i32::try_from(receipt.source_file_count).unwrap_or(i32::MAX))
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn fail(&self, organization: &str, request_id: &str) -> Result<(), ExportError> {
        if request_id.is_empty() {
            return Ok(());
        }
        let mut transaction = self.control.begin_tenant(organization).await?;
        let changed = sqlx::query(
            r"UPDATE compliance.export_requests SET status='failed',
            error_code='privacy.export_failed',completed_at=clock_timestamp()
            WHERE id=$1::uuid AND status='running'",
        )
        .bind(request_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed == 1 {
            sqlx::query(r"INSERT INTO control.audit_log
                (organization_id,action,target_type,target_id,request_id,reason_code)
                VALUES ($1::uuid,'privacy.export.failed','export_request',$2::uuid,$2,'privacy.export_failed')")
                .bind(organization).bind(request_id).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

fn same_request(request: &ExportRequest, row: &RequestRow, digest: Option<&[u8; 32]>) -> bool {
    row.1.as_deref() == nonempty(&request.project_id)
        && row.2.as_deref() == nonempty(&request.environment_id)
        && row.3 == request.kind.as_str()
        && row.4.as_deref() == request.target_kind.map(ExportTargetKind::as_str)
        && row.5.as_deref() == digest.map(<[u8; 32]>::as_slice)
        && row.6 == request.requested_by
        && row.7 == request.reason_code
}

async fn ensure_no_uncommitted_matching_records(
    transaction: &mut Transaction<'_, Postgres>,
    request: &ExportRequest,
) -> Result<(), ExportError> {
    let uncommitted: i64 = match request.kind {
        ExportKind::Tenant => {
            sqlx::query_scalar(
                r"SELECT count(*) FROM ingest.canonical_envelopes AS envelope
              WHERE envelope.organization_id=$1::uuid AND NOT EXISTS (
                SELECT 1 FROM lake.batch_records AS member
                JOIN lake.export_batches AS batch
                  ON batch.batch_id=member.batch_id
                 AND batch.organization_id=member.organization_id
                 AND batch.project_id=member.project_id
                 AND batch.environment_id=member.environment_id
                WHERE member.canonical_envelope_id=envelope.id
                  AND member.organization_id=envelope.organization_id
                  AND member.project_id=envelope.project_id
                  AND member.environment_id=envelope.environment_id
                  AND batch.status='committed')",
            )
            .bind(&request.organization_id)
            .fetch_one(&mut **transaction)
            .await?
        }
        ExportKind::DataSubject => {
            let target_column = export_target_column(request.target_kind.ok_or(
                ExportError::InvalidRequest("subject export target kind is required"),
            )?);
            let query = format!(
                r"SELECT count(*) FROM ingest.canonical_envelopes AS envelope
                  WHERE envelope.organization_id=$1::uuid
                    AND envelope.project_id=$2::uuid
                    AND envelope.environment_id=$3::uuid
                    AND envelope.{target_column}=$4
                    AND NOT EXISTS (
                      SELECT 1 FROM lake.batch_records AS member
                      JOIN lake.export_batches AS batch
                        ON batch.batch_id=member.batch_id
                       AND batch.organization_id=member.organization_id
                       AND batch.project_id=member.project_id
                       AND batch.environment_id=member.environment_id
                      WHERE member.canonical_envelope_id=envelope.id
                        AND member.organization_id=envelope.organization_id
                        AND member.project_id=envelope.project_id
                        AND member.environment_id=envelope.environment_id
                        AND batch.status='committed')"
            );
            sqlx::query_scalar(sqlx::AssertSqlSafe(query.as_str()))
                .bind(&request.organization_id)
                .bind(&request.project_id)
                .bind(&request.environment_id)
                .bind(&request.target_value)
                .fetch_one(&mut **transaction)
                .await?
        }
    };
    if uncommitted != 0 {
        return Err(ExportError::State(format!(
            "{uncommitted} matching canonical records are not yet committed to the lake"
        )));
    }
    Ok(())
}

const fn export_target_column(target: ExportTargetKind) -> &'static str {
    match target {
        ExportTargetKind::InstallationId => "installation_id",
        ExportTargetKind::SessionId => "session_id",
        ExportTargetKind::ReplayId => "replay_id",
    }
}

fn build_archive(
    receipt: &ExportReceipt,
    request: &ExportRequest,
    rows: &[Row],
    policies: &[PolicyRecord],
    audits: &[AuditRecord],
) -> Result<Vec<u8>, ExportError> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = ZipWriter::new(cursor);
    let created_at = receipt
        .created_at
        .format(&Rfc3339)
        .map_err(|error| ExportError::State(format!("format creation time: {error}")))?;
    let manifest = json!({ "format": "chill-privacy-export-v1", "request_id": receipt.request_id,
        "created_at": created_at, "kind": request.kind, "organization_id": request.organization_id,
        "project_id": request.project_id, "environment_id": request.environment_id,
        "target_kind": request.target_kind, "record_count": rows.len(),
        "collection_policy_count": policies.len(), "audit_record_count": audits.len() });
    let mut manifest_body = serde_json::to_vec_pretty(&manifest)?;
    manifest_body.push(b'\n');
    write_entry(&mut archive, "manifest.json", &manifest_body)?;
    let canonical: Vec<Value> = rows
        .iter()
        .map(|row| serde_json::from_str(&row.canonical_json))
        .collect::<Result<_, _>>()?;
    write_ndjson(&mut archive, "records.ndjson", &canonical)?;
    write_ndjson(&mut archive, "collection-policies.ndjson", policies)?;
    write_ndjson(&mut archive, "audit.ndjson", audits)?;
    Ok(archive.finish()?.into_inner())
}

fn write_ndjson<T: Serialize>(
    archive: &mut ZipWriter<Cursor<Vec<u8>>>,
    name: &str,
    values: &[T],
) -> Result<(), ExportError> {
    let mut body = Vec::new();
    for value in values {
        serde_json::to_writer(&mut body, value)?;
        body.push(b'\n');
    }
    write_entry(archive, name, &body)
}

fn write_entry(
    archive: &mut ZipWriter<Cursor<Vec<u8>>>,
    name: &str,
    body: &[u8],
) -> Result<(), ExportError> {
    let timestamp = zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
        .map_err(|error| ExportError::State(format!("build ZIP timestamp: {error}")))?;
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(timestamp)
        .unix_permissions(0o600);
    archive.start_file(name, options)?;
    archive.write_all(body)?;
    Ok(())
}

fn canonical_uuid(value: &str) -> Result<(), ExportError> {
    let parsed =
        Uuid::parse_str(value).map_err(|_| ExportError::InvalidRequest("scope UUID is invalid"))?;
    if parsed.to_string() != value {
        return Err(ExportError::InvalidRequest("scope UUID is not canonical"));
    }
    Ok(())
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    fn request() -> ExportRequest {
        ExportRequest {
            organization_id: "00000000-0000-4000-8000-000000000001".to_owned(),
            project_id: "00000000-0000-4000-8000-000000000002".to_owned(),
            environment_id: "00000000-0000-4000-8000-000000000003".to_owned(),
            kind: ExportKind::DataSubject,
            target_kind: Some(ExportTargetKind::InstallationId),
            target_value: "private-installation".to_owned(),
            requested_by: "privacy@example.com".to_owned(),
            reason_code: "privacy.user_export".to_owned(),
            idempotency_key: "user-export-1".to_owned(),
        }
    }

    #[test]
    fn validation_and_matching_fail_closed() {
        let valid = request();
        assert!(valid.validate().is_ok());
        let mut invalid = valid.clone();
        invalid.kind = ExportKind::Tenant;
        assert!(invalid.validate().is_err());
        let row = Row {
            installation_id: Some(valid.target_value.clone()),
            session_id: Some("session".to_owned()),
            ..row()
        };
        assert!(valid.matches(&row));
        let mut wrong = valid;
        wrong.target_kind = Some(ExportTargetKind::SessionId);
        assert!(!wrong.matches(&row));
    }

    #[test]
    fn archive_is_private_and_omits_subject_from_manifest() {
        let request = request();
        let receipt = ExportReceipt {
            request_id: "00000000-0000-4000-8000-000000000004".to_owned(),
            artifact_sha256: String::new(),
            record_count: 0,
            artifact_bytes: 0,
            source_file_count: 0,
            created_at: OffsetDateTime::UNIX_EPOCH,
        };
        let body = build_archive(&receipt, &request, &[row()], &[], &[])
            .unwrap_or_else(|error| unreachable!("test archive: {error}"));
        let mut archive = zip::ZipArchive::new(Cursor::new(body))
            .unwrap_or_else(|error| unreachable!("read test archive: {error}"));
        assert_eq!(archive.len(), 4);
        let mut manifest = String::new();
        let mut file = archive
            .by_name("manifest.json")
            .unwrap_or_else(|error| unreachable!("manifest entry: {error}"));
        assert_eq!(file.unix_mode().map(|mode| mode & 0o777), Some(0o600));
        file.read_to_string(&mut manifest)
            .unwrap_or_else(|error| unreachable!("manifest body: {error}"));
        assert!(!manifest.contains(&request.target_value));
    }

    fn row() -> Row {
        Row {
            dataset_schema_version: "1".to_owned(),
            batch_id: "0".repeat(64),
            row_ordinal: 0,
            canonical_envelope_id: 1,
            organization_id: String::new(),
            project_id: String::new(),
            environment_id: String::new(),
            data_source_id: String::new(),
            envelope_version: "1.0.0".to_owned(),
            envelope_kind: "behavior".to_owned(),
            record_id: "record".to_owned(),
            record_sha256: "0".repeat(64),
            installation_id: None,
            session_id: None,
            replay_id: None,
            replay_chunk_id: None,
            trace_id: None,
            span_id: None,
            occurred_at_unix_nano: None,
            source_observed_at_unix_nano: None,
            server_received_at_unix_nano: 1,
            effective_occurred_at_unix_nano: 1,
            monotonic_nano: None,
            boot_id: None,
            sequence_number: None,
            clock_skew_nano: 0,
            timing_class: "source".to_owned(),
            late_arrival: false,
            canonical_json: "{\"record\":\"one\"}".to_owned(),
            normalized_at_unix_nano: 1,
        }
    }
}
