use std::sync::LazyLock;

use chill_lake::Row;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

static REASON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$")
        .unwrap_or_else(|error| unreachable!("static reason regex: {error}"))
});

/// Deletion policy class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// General environment retention.
    Retention,
    /// Replay-specific retention.
    ReplayExpiry,
    /// One installation/session/replay subject.
    DataSubject,
    /// Entire environment.
    Environment,
    /// Entire organization.
    Tenant,
}

impl Kind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Retention => "retention",
            Self::ReplayExpiry => "replay_expiry",
            Self::DataSubject => "data_subject",
            Self::Environment => "environment",
            Self::Tenant => "tenant",
        }
    }
}

/// Data-subject identifier column.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// Installation identifier.
    InstallationId,
    /// Session identifier.
    SessionId,
    /// Replay identifier.
    ReplayId,
}

impl TargetKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InstallationId => "installation_id",
            Self::SessionId => "session_id",
            Self::ReplayId => "replay_id",
        }
    }
}

/// Rejected lifecycle request.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("lifecycle deletion request is invalid: {0}")]
pub struct RequestError(&'static str);

/// Durable deletion request and lease state.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    /// Database UUID after creation.
    #[serde(default)]
    pub id: String,
    /// Organization UUID.
    pub organization_id: String,
    /// Project UUID except for tenant deletion.
    #[serde(default)]
    pub project_id: String,
    /// Environment UUID except for tenant deletion.
    #[serde(default)]
    pub environment_id: String,
    /// Policy class.
    pub kind: Kind,
    /// Subject identifier kind.
    pub target_kind: Option<TargetKind>,
    /// Raw target retained only until completion.
    #[serde(skip)]
    pub target_value: String,
    /// Target digest persisted for idempotency and tombstones.
    #[serde(skip)]
    pub target_sha256: [u8; 32],
    /// Retention cutoff in Unix nanoseconds.
    pub cutoff_unix_nano: Option<u64>,
    /// Stable actor or worker identity.
    pub requested_by: String,
    /// Auditable reason code.
    pub reason_code: String,
    /// Tenant-scoped idempotency key.
    pub idempotency_key: String,
    /// Durable lease attempt count.
    #[serde(default)]
    pub attempt_count: i32,
    /// Creation time after persistence.
    #[serde(skip)]
    pub created_at: Option<OffsetDateTime>,
}

impl Request {
    /// Validates scope, target, cutoff, actor, reason, and idempotency bounds.
    ///
    /// # Errors
    ///
    /// Returns an error if the request is ambiguous, malformed, or unbounded.
    pub fn validate(&self) -> Result<(), RequestError> {
        canonical_uuid(&self.organization_id)?;
        if self.kind == Kind::Tenant {
            if !self.project_id.is_empty() || !self.environment_id.is_empty() {
                return Err(RequestError("tenant scope includes project or environment"));
            }
        } else {
            canonical_uuid(&self.project_id)?;
            canonical_uuid(&self.environment_id)?;
        }
        match self.kind {
            Kind::Retention | Kind::ReplayExpiry => {
                if self.cutoff_unix_nano.is_none()
                    || self.target_kind.is_some()
                    || !self.target_value.is_empty()
                {
                    return Err(RequestError("retention scope or cutoff is invalid"));
                }
            }
            Kind::DataSubject => {
                if self.target_kind.is_none()
                    || self.target_value.is_empty()
                    || self.target_value.len() > 256
                    || self.target_value.trim() != self.target_value
                    || self.cutoff_unix_nano.is_some()
                {
                    return Err(RequestError("data-subject target is invalid"));
                }
            }
            Kind::Environment | Kind::Tenant => {
                if self.target_kind.is_some()
                    || !self.target_value.is_empty()
                    || self.cutoff_unix_nano.is_some()
                {
                    return Err(RequestError("scope deletion contains target or cutoff"));
                }
            }
        }
        if self.requested_by.is_empty()
            || self.requested_by.len() > 160
            || !REASON.is_match(&self.reason_code)
            || self.idempotency_key.is_empty()
            || self.idempotency_key.len() > 128
        {
            return Err(RequestError("audit or idempotency fields are invalid"));
        }
        Ok(())
    }

    pub(crate) fn target_digest(&self) -> [u8; 32] {
        if self.kind == Kind::DataSubject {
            Sha256::digest(self.target_value.as_bytes()).into()
        } else {
            [0; 32]
        }
    }

    pub(crate) fn matches(&self, row: &Row) -> bool {
        match self.kind {
            Kind::Retention => self
                .cutoff_unix_nano
                .is_some_and(|cutoff| row.effective_occurred_at_unix_nano < cutoff),
            Kind::ReplayExpiry => {
                row.envelope_kind == "replay"
                    && self
                        .cutoff_unix_nano
                        .is_some_and(|cutoff| row.effective_occurred_at_unix_nano < cutoff)
            }
            Kind::DataSubject => match self.target_kind {
                Some(TargetKind::InstallationId) => row.installation_id.as_deref(),
                Some(TargetKind::SessionId) => row.session_id.as_deref(),
                Some(TargetKind::ReplayId) => row.replay_id.as_deref(),
                None => None,
            }
            .is_some_and(|value| value == self.target_value),
            Kind::Environment | Kind::Tenant => true,
        }
    }
}

/// Auditable deletion completion counters and digest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Completion {
    /// Request UUID.
    pub request_id: String,
    /// Canonical rows deleted.
    pub deleted_rows: i64,
    /// Raw inbox bytes deleted.
    pub deleted_inbox_bytes: i64,
    /// Immutable object bytes deleted.
    pub deleted_object_bytes: i64,
    /// Replacement Parquet files written.
    pub rewritten_files: i32,
    /// Source Parquet files deleted.
    pub deleted_files: i32,
    /// SHA-256 of stable completion data.
    pub completion_sha256: String,
}

fn canonical_uuid(value: &str) -> Result<(), RequestError> {
    let parsed = Uuid::parse_str(value).map_err(|_| RequestError("scope UUID is invalid"))?;
    if parsed.to_string() != value {
        return Err(RequestError("scope UUID is not canonical"));
    }
    Ok(())
}
