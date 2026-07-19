use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::Date;

/// Stable lake dataset schema.
pub const DATASET_SCHEMA_VERSION: &str = "1";
/// Writer implementation included in batch identity and manifests.
pub const WRITER_FORMAT_VERSION: &str = "arrow-rs-59.1.0-zstd-default-v1";

/// Lake encoding or identity failure.
#[derive(Debug, Error)]
pub enum LakeError {
    /// A batch or row violates a partition/identity contract.
    #[error("lake identity is invalid: {0}")]
    Invalid(String),
    /// Arrow construction failed.
    #[error("build Arrow batch: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// Parquet encoding failed.
    #[error("encode Parquet: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// Manifest JSON encoding failed.
    #[error("encode lake manifest: {0}")]
    Json(#[from] serde_json::Error),
}

/// One row in the versioned lake dataset.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Dataset schema version, assigned during encoding.
    pub dataset_schema_version: String,
    /// Owning batch identity, assigned during encoding.
    pub batch_id: String,
    /// Stable row ordinal, assigned after source-ID sorting.
    pub row_ordinal: i32,
    /// Source canonical-envelope identity.
    pub canonical_envelope_id: i64,
    /// Organization UUID.
    pub organization_id: String,
    /// Project UUID.
    pub project_id: String,
    /// Environment UUID.
    pub environment_id: String,
    /// Data-source UUID.
    pub data_source_id: String,
    /// Canonical envelope version.
    pub envelope_version: String,
    /// Canonical envelope kind.
    pub envelope_kind: String,
    /// Stable record identity.
    pub record_id: String,
    /// Lowercase source SHA-256.
    pub record_sha256: String,
    /// Optional installation identity.
    pub installation_id: Option<String>,
    /// Optional session identity.
    pub session_id: Option<String>,
    /// Optional replay identity.
    pub replay_id: Option<String>,
    /// Optional replay chunk identity.
    pub replay_chunk_id: Option<String>,
    /// Optional trace identity.
    pub trace_id: Option<String>,
    /// Optional span identity.
    pub span_id: Option<String>,
    /// Source occurrence time.
    pub occurred_at_unix_nano: Option<u64>,
    /// Source observation time.
    pub source_observed_at_unix_nano: Option<u64>,
    /// Trusted server receipt time.
    pub server_received_at_unix_nano: u64,
    /// Effective query/retention time.
    pub effective_occurred_at_unix_nano: u64,
    /// Optional source monotonic time.
    pub monotonic_nano: Option<u64>,
    /// Optional source boot epoch.
    pub boot_id: Option<String>,
    /// Optional source sequence.
    pub sequence_number: Option<u64>,
    /// Signed server/source skew.
    pub clock_skew_nano: i64,
    /// Stable timing class.
    pub timing_class: String,
    /// Whether this arrived after the late threshold.
    pub late_arrival: bool,
    /// Canonical envelope JSON.
    pub canonical_json: String,
    /// Normalization timestamp.
    pub normalized_at_unix_nano: i64,
}

/// One deterministic micro, compaction, or lifecycle rewrite batch.
#[derive(Clone, Debug)]
pub struct Batch {
    /// SHA-256 batch identity.
    pub id: String,
    /// Organization UUID.
    pub organization_id: String,
    /// Project UUID.
    pub project_id: String,
    /// Environment UUID.
    pub environment_id: String,
    /// `micro`, `compaction`, or `rewrite`.
    pub kind: String,
    /// UTC partition day.
    pub partition_day: Date,
    /// UTC partition hour.
    pub partition_hour: u8,
    /// Canonical envelope kind.
    pub envelope_kind: String,
    /// Number of rows.
    pub row_count: usize,
    /// Minimum server receipt time.
    pub min_server_received_at_unix_nano: u64,
    /// Maximum server receipt time.
    pub max_server_received_at_unix_nano: u64,
    /// Minimum effective occurrence time.
    pub min_effective_occurred_at_unix_nano: u64,
    /// Maximum effective occurrence time.
    pub max_effective_occurred_at_unix_nano: u64,
    /// Current attempt count.
    pub attempt_count: i32,
    /// Source batches superseded by this batch.
    pub supersedes: Vec<String>,
}

impl Batch {
    /// Validates the complete batch identity and range contract.
    ///
    /// # Errors
    ///
    /// Rejects malformed digest, tenant scope, kind, partition, counts, or time ranges.
    pub fn validate(&self) -> Result<(), LakeError> {
        let digest = hex::decode(&self.id).map_err(|_| invalid("batch ID is not hexadecimal"))?;
        if digest.len() != 32 || self.id != self.id.to_ascii_lowercase() {
            return Err(invalid("batch ID must be a lowercase SHA-256"));
        }
        if self.organization_id.is_empty()
            || self.project_id.is_empty()
            || self.environment_id.is_empty()
            || !matches!(self.kind.as_str(), "micro" | "compaction" | "rewrite")
            || !matches!(
                self.envelope_kind.as_str(),
                "behavior" | "otel.log" | "otel.span" | "otel.metric" | "replay"
            )
            || self.partition_hour > 23
            || self.row_count == 0
            || self.min_server_received_at_unix_nano > self.max_server_received_at_unix_nano
            || self.min_effective_occurred_at_unix_nano > self.max_effective_occurred_at_unix_nano
        {
            return Err(invalid(
                "batch scope, partition, count, or range is invalid",
            ));
        }
        Ok(())
    }

    /// Returns the safe object partition prefix.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch is invalid.
    pub fn prefix(&self) -> Result<String, LakeError> {
        self.validate()?;
        Ok(format!(
            "v1/organization={}/project={}/environment={}/day={}/hour={:02}/kind={}",
            self.organization_id,
            self.project_id,
            self.environment_id,
            self.partition_day,
            self.partition_hour,
            self.envelope_kind
        ))
    }

    /// Returns the immutable Parquet object key.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch is invalid.
    pub fn object_key(&self) -> Result<String, LakeError> {
        Ok(format!("{}/part-{}.parquet", self.prefix()?, self.id))
    }

    /// Returns the immutable manifest key.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch is invalid.
    pub fn manifest_key(&self) -> Result<String, LakeError> {
        Ok(format!("{}/manifest-{}.json", self.prefix()?, self.id))
    }
}

/// Result of immutable object publication.
#[derive(Clone, Debug)]
pub struct Published {
    /// Parquet object key.
    pub object_key: String,
    /// Manifest object key.
    pub manifest_key: String,
    /// Parquet SHA-256.
    pub object_digest: [u8; 32],
    /// Manifest SHA-256.
    pub manifest_digest: [u8; 32],
    /// Parquet byte count.
    pub byte_count: i64,
}

/// Public immutable lake manifest.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    /// Manifest media contract.
    pub format: String,
    /// Dataset schema version.
    pub dataset_schema_version: String,
    /// Writer implementation version.
    pub writer_format_version: String,
    /// Batch identity.
    pub batch_id: String,
    /// Batch kind.
    pub batch_kind: String,
    /// Organization UUID.
    pub organization_id: String,
    /// Project UUID.
    pub project_id: String,
    /// Environment UUID.
    pub environment_id: String,
    /// UTC partition day.
    pub partition_day: String,
    /// UTC partition hour.
    pub partition_hour: u8,
    /// Envelope kind.
    pub envelope_kind: String,
    /// Parquet object key.
    pub object_key: String,
    /// Lowercase Parquet SHA-256.
    pub object_sha256: String,
    /// Parquet byte count.
    pub byte_count: i64,
    /// Row count.
    pub row_count: usize,
    /// Minimum server receipt time.
    pub min_server_received_at_unix_nano: u64,
    /// Maximum server receipt time.
    pub max_server_received_at_unix_nano: u64,
    /// Minimum effective occurrence time.
    pub min_effective_occurred_at_unix_nano: u64,
    /// Maximum effective occurrence time.
    pub max_effective_occurred_at_unix_nano: u64,
    /// Sorted source batch identities.
    pub supersedes: Vec<String>,
}

/// Computes a deterministic batch identity over schema, writer, partition, records, and sources.
#[must_use]
pub fn deterministic_batch_id(
    kind: &str,
    partition_key: &str,
    record_digests: &[Vec<u8>],
    source_batch_ids: &[String],
) -> String {
    let mut digest = Sha256::new();
    for value in [
        DATASET_SCHEMA_VERSION,
        WRITER_FORMAT_VERSION,
        kind,
        partition_key,
    ] {
        digest.update(value.len().to_string());
        digest.update(b":");
        digest.update(value.as_bytes());
    }
    for value in record_digests {
        digest.update(value.len().to_string());
        digest.update(b":");
        digest.update(value);
    }
    for source in source_batch_ids.iter().collect::<BTreeSet<_>>() {
        digest.update(source.as_bytes());
    }
    hex::encode(digest.finalize())
}

pub(crate) fn invalid(message: impl Into<String>) -> LakeError {
    LakeError::Invalid(message.into())
}
