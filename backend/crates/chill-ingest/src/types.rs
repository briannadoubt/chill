use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::digest::Output;
use thiserror::Error;
use time::OffsetDateTime;

/// Ingestion signal namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalKind {
    /// OTLP logs.
    Logs,
    /// OTLP traces.
    Traces,
    /// OTLP metrics.
    Metrics,
    /// Encrypted Chill replay chunk.
    Replay,
}

impl SignalKind {
    /// Returns the stable database value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Logs => "logs",
            Self::Traces => "traces",
            Self::Metrics => "metrics",
            Self::Replay => "replay",
        }
    }
}

/// Wire encoding stored with the durable payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum PayloadFormat {
    /// OTLP protobuf.
    #[serde(rename = "protobuf")]
    Protobuf,
    /// OTLP JSON.
    #[serde(rename = "json")]
    Json,
    /// Encrypted replay envelope v1.
    #[serde(rename = "replay-v1")]
    ReplayV1,
}

impl PayloadFormat {
    /// Returns the stable database value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Protobuf => "protobuf",
            Self::Json => "json",
            Self::ReplayV1 => "replay-v1",
        }
    }
}

/// Untrusted transport candidate.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// Signal namespace.
    pub kind: SignalKind,
    /// Wire encoding.
    pub format: PayloadFormat,
    /// Bounded request body.
    pub payload: Vec<u8>,
    /// Raw SDK bearer credential.
    pub credential: String,
    /// Optional caller-supplied idempotency key.
    pub idempotency_key: String,
    /// Trusted replay headers, when applicable.
    pub replay: Option<ReplayMetadata>,
}

/// Trusted metadata for an encrypted replay chunk.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReplayMetadata {
    /// Replay ID.
    pub replay_id: String,
    /// Chunk ID.
    pub chunk_id: String,
    /// Session ID.
    pub session_id: String,
    /// Boot ID.
    pub boot_id: String,
    /// Chunk start on the monotonic clock.
    pub start_monotonic_nano: u64,
    /// Chunk end on the monotonic clock.
    pub end_monotonic_nano: u64,
    /// Source wall-clock occurrence time.
    pub occurred_at_unix_nano: u64,
    /// Lowercase encrypted-payload SHA-256.
    pub digest: String,
    /// Replay envelope codec.
    pub codec: String,
    /// Optional W3C trace context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
}

/// Behavior schema referenced by a Chill log record.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SchemaRef {
    /// Schema version.
    pub version: String,
    /// Canonical schema URL.
    pub url: String,
}

/// Fully validated request ready for authorization and durable admission.
#[derive(Clone, Debug)]
pub struct ValidatedRequest {
    /// Signal namespace.
    pub kind: SignalKind,
    /// Wire encoding.
    pub format: PayloadFormat,
    /// Immutable payload bytes.
    pub payload: Vec<u8>,
    /// SHA-256 of payload bytes.
    pub payload_digest: Output<sha2::Sha256>,
    /// Normalized idempotency key.
    pub idempotency_key: String,
    /// Validated logical record count.
    pub record_count: usize,
    /// Referenced behavior schemas.
    pub schema_refs: Vec<SchemaRef>,
    /// Trusted replay metadata.
    pub replay: Option<ReplayMetadata>,
}

/// Durable admission receipt.
#[derive(Clone, Debug)]
pub struct Receipt {
    /// Inbox identity.
    pub inbox_id: i64,
    /// Effective idempotency key.
    pub idempotency_key: String,
    /// Whether an identical prior request was returned.
    pub duplicate: bool,
    /// Database receipt time.
    pub server_received_at: OffsetDateTime,
}

/// Stable transport-independent admission failure code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    /// Invalid input.
    Invalid,
    /// Invalid credential.
    Unauthorized,
    /// Insufficient SDK scope or tenant state.
    Forbidden,
    /// Body limit exceeded.
    TooLarge,
    /// Idempotency key reused for different content.
    Conflict,
    /// Environment quota exhausted.
    Quota,
    /// Process admission slots exhausted.
    Backpressure,
    /// Transient dependency failure.
    Unavailable,
}

/// Transport-independent admission failure.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct AdmissionError {
    /// Stable failure code.
    pub code: ErrorCode,
    /// Safe caller-facing message.
    pub message: String,
    /// Optional bounded retry delay.
    pub retry_after: Option<Duration>,
    /// Internal diagnostic source.
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl AdmissionError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after: None,
            source: None,
        }
    }

    pub(crate) fn invalid(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            code: ErrorCode::Invalid,
            message: "request payload does not satisfy the ingestion schema".to_owned(),
            retry_after: None,
            source: Some(Box::new(source)),
        }
    }

    pub(crate) fn with_source(
        code: ErrorCode,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after: None,
            source: Some(Box::new(source)),
        }
    }

    pub(crate) fn retry(
        code: ErrorCode,
        message: impl Into<String>,
        retry_after: Duration,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after: Some(retry_after),
            source: None,
        }
    }
}

/// Process-level ingestion limits.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum decompressed OTLP bytes.
    pub maximum_payload_bytes: usize,
    /// Maximum encrypted replay bytes.
    pub maximum_replay_bytes: usize,
    /// Maximum logical records per request.
    pub maximum_records: usize,
    /// Maximum concurrent admissions.
    pub maximum_concurrent: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            maximum_payload_bytes: 4 << 20,
            maximum_replay_bytes: 8 << 20,
            maximum_records: 10_000,
            maximum_concurrent: 32,
        }
    }
}

impl Limits {
    /// Validates configured safety bounds.
    ///
    /// # Errors
    ///
    /// Returns invalid input when any bound is outside the production envelope.
    pub fn validate(self) -> Result<(), AdmissionError> {
        if !(1024..=16 << 20).contains(&self.maximum_payload_bytes)
            || !(1024..=16 << 20).contains(&self.maximum_replay_bytes)
            || !(1..=1_000_000).contains(&self.maximum_records)
            || !(1..=4096).contains(&self.maximum_concurrent)
        {
            return Err(AdmissionError::new(
                ErrorCode::Invalid,
                "ingestion limits are outside the production envelope",
            ));
        }
        Ok(())
    }
}
