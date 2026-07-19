use chill_control_plane::{ControlPlaneError, Store};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::OffsetDateTime;

use crate::{Plan, QueryError, Scope};

/// One immutable committed lake object selected by the catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LakeFile {
    /// Deterministic export batch identity.
    pub batch_id: String,
    /// Immutable object-store key.
    pub object_key: String,
    /// Expected SHA-256 object digest.
    pub digest: [u8; 32],
    /// Compressed object bytes.
    pub byte_count: i64,
    /// Declared row count.
    pub row_count: i32,
    /// Commit publication time.
    pub published_at: OffsetDateTime,
}

/// Tenant-safe immutable input snapshot for one query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dataset {
    /// Ordered committed objects.
    pub files: Vec<LakeFile>,
    /// Digest of policies, lifecycle generation, batch IDs, and object digests.
    pub generation: String,
    /// Active policy version and digest.
    pub policy_version: String,
    /// Environment lifecycle generation.
    pub lifecycle_generation: i64,
    /// Total compressed input bytes.
    pub byte_count: i64,
    /// Total declared rows.
    pub row_count: i64,
}

/// Catalog resolution failure.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// Scope or plan validation failed.
    #[error("invalid query catalog input: {0}")]
    Invalid(#[from] QueryError),
    /// Tenant transaction setup failed.
    #[error("query catalog tenant operation failed: {0}")]
    Control(#[from] ControlPlaneError),
    /// Database operation failed.
    #[error("query catalog database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// The exact environment has no active privacy policy.
    #[error("query scope is unavailable")]
    ScopeNotFound,
    /// Committed metadata violated the lake contract.
    #[error("committed lake metadata is invalid")]
    InvalidMetadata,
}

/// PostgreSQL-backed committed lake catalog.
#[derive(Clone)]
pub struct Catalog {
    control: Store,
}

impl Catalog {
    /// Creates a tenant-safe catalog.
    #[must_use]
    pub fn new(control: Store) -> Self {
        Self { control }
    }

    /// Resolves the exact committed, non-superseded inputs overlapping a plan.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid scope, absent policy, tenant/database
    /// failures, or malformed committed metadata.
    pub async fn resolve(&self, scope: &Scope, plan: &Plan) -> Result<Dataset, CatalogError> {
        scope.validate()?;
        let mut transaction = self.control.begin_tenant(&scope.organization_id).await?;
        let lifecycle_generation = sqlx::query_scalar::<_, i64>(
            r"SELECT coalesce((SELECT generation FROM lifecycle.environment_generations
              WHERE organization_id=$1::uuid AND project_id=$2::uuid AND environment_id=$3::uuid),0)",
        )
        .bind(&scope.organization_id)
        .bind(&scope.project_id)
        .bind(&scope.environment_id)
        .fetch_one(&mut *transaction)
        .await?;
        let policy = sqlx::query_as::<_, (i64, Vec<u8>)>(
            r"SELECT version,digest FROM control.privacy_policies WHERE organization_id=$1::uuid
              AND project_id=$2::uuid AND environment_id=$3::uuid AND status='active'
              ORDER BY version DESC LIMIT 1",
        )
        .bind(&scope.organization_id)
        .bind(&scope.project_id)
        .bind(&scope.environment_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(CatalogError::ScopeNotFound)?;
        let policy_digest: [u8; 32] = policy
            .1
            .try_into()
            .map_err(|_| CatalogError::InvalidMetadata)?;
        let policy_version = format!("privacy:{}:{}", policy.0, hex::encode(policy_digest));
        let kinds: Vec<String> = plan
            .envelope_kinds()
            .iter()
            .map(ToString::to_string)
            .collect();
        let rows = sqlx::query_as::<_, LakeFileRow>(
            r"SELECT batch_id,object_key,object_sha256,byte_count,row_count,published_at
              FROM lake.export_batches WHERE organization_id=$1::uuid AND project_id=$2::uuid
              AND environment_id=$3::uuid AND status='committed' AND envelope_kind=ANY($4)
              AND max_effective_occurred_at_unix_nano >= $5::numeric
              AND min_effective_occurred_at_unix_nano < $6::numeric
              ORDER BY partition_day,partition_hour,envelope_kind,batch_id",
        )
        .bind(&scope.organization_id)
        .bind(&scope.project_id)
        .bind(&scope.environment_id)
        .bind(kinds)
        .bind(plan.range.start_unix_nano.to_string())
        .bind(plan.range.end_unix_nano.to_string())
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        let mut files = Vec::with_capacity(rows.len());
        let mut byte_count = 0_i64;
        let mut row_count = 0_i64;
        for row in rows {
            let digest: [u8; 32] = row
                .object_sha256
                .try_into()
                .map_err(|_| CatalogError::InvalidMetadata)?;
            if row.object_key.is_empty() || row.byte_count < 1 || row.row_count < 1 {
                return Err(CatalogError::InvalidMetadata);
            }
            byte_count = byte_count
                .checked_add(row.byte_count)
                .ok_or(CatalogError::InvalidMetadata)?;
            row_count = row_count
                .checked_add(i64::from(row.row_count))
                .ok_or(CatalogError::InvalidMetadata)?;
            files.push(LakeFile {
                batch_id: row.batch_id,
                object_key: row.object_key,
                digest,
                byte_count: row.byte_count,
                row_count: row.row_count,
                published_at: row.published_at,
            });
        }
        let mut generation = Sha256::new();
        generation.update(policy_version.as_bytes());
        generation.update(lifecycle_generation.to_string().as_bytes());
        for file in &files {
            generation.update(file.batch_id.as_bytes());
            generation.update(file.digest);
        }
        Ok(Dataset {
            files,
            generation: hex::encode(generation.finalize()),
            policy_version,
            lifecycle_generation,
            byte_count,
            row_count,
        })
    }
}

#[derive(sqlx::FromRow)]
struct LakeFileRow {
    batch_id: String,
    object_key: String,
    object_sha256: Vec<u8>,
    byte_count: i64,
    row_count: i32,
    published_at: OffsetDateTime,
}
