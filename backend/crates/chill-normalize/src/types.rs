use serde_json::Value;
use time::OffsetDateTime;

use crate::Timing;

/// One leased durable ingestion payload.
#[derive(Clone, Debug)]
pub struct InboxItem {
    /// Inbox identity.
    pub id: i64,
    /// Owning organization UUID.
    pub organization_id: String,
    /// Owning project UUID.
    pub project_id: String,
    /// Owning environment UUID.
    pub environment_id: String,
    /// Originating data-source UUID.
    pub data_source_id: String,
    /// Stable signal value.
    pub signal_kind: String,
    /// Stable wire format.
    pub payload_format: String,
    /// Immutable admitted payload.
    pub payload: Vec<u8>,
    /// Trusted server-side metadata.
    pub metadata: Value,
    /// Database receipt timestamp.
    pub server_received_at: OffsetDateTime,
    /// Current delivery attempt.
    pub attempt_count: i32,
}

/// One canonical envelope ready for persistence.
#[derive(Clone, Debug)]
pub struct Record {
    /// Stable ordinal within the inbox payload.
    pub ordinal: i32,
    /// Canonical envelope namespace.
    pub envelope_kind: String,
    /// Stable record identity.
    pub record_id: String,
    /// SHA-256 of immutable source content.
    pub digest: [u8; 32],
    /// Optional installation identity.
    pub installation_id: Option<String>,
    /// Optional session identity.
    pub session_id: Option<String>,
    /// Optional replay identity.
    pub replay_id: Option<String>,
    /// Optional replay chunk identity.
    pub replay_chunk_id: Option<String>,
    /// Optional lowercase trace identity.
    pub trace_id: Option<String>,
    /// Optional lowercase span identity.
    pub span_id: Option<String>,
    /// Source occurrence timestamp.
    pub occurred_at_unix_nano: Option<u64>,
    /// Source observation timestamp.
    pub source_observed_at_unix_nano: Option<u64>,
    /// Source monotonic timestamp.
    pub monotonic_nano: Option<u64>,
    /// Source boot epoch.
    pub boot_id: Option<String>,
    /// Source ordering sequence.
    pub sequence_number: Option<u64>,
    /// Server timing classification.
    pub timing: Timing,
    /// Canonical JSON document.
    pub canonical: Value,
}
