use serde::Serialize;

/// Server-authorized operations exposed to users and service credentials.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Capability {
    /// Read configuration metadata.
    #[serde(rename = "control:read")]
    ControlRead,
    /// Mutate configuration metadata.
    #[serde(rename = "control:write")]
    ControlWrite,
    /// Create, rotate, and revoke credentials.
    #[serde(rename = "credentials:manage")]
    CredentialsManage,
    /// Query tenant data.
    #[serde(rename = "data:read")]
    DataRead,
    /// Delete tenant data.
    #[serde(rename = "data:delete")]
    DataDelete,
}

impl Capability {
    /// Parses a stable database and protocol scope value.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "control:read" => Some(Self::ControlRead),
            "control:write" => Some(Self::ControlWrite),
            "credentials:manage" => Some(Self::CredentialsManage),
            "data:read" => Some(Self::DataRead),
            "data:delete" => Some(Self::DataDelete),
            _ => None,
        }
    }

    /// Returns the stable database and protocol scope value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ControlRead => "control:read",
            Self::ControlWrite => "control:write",
            Self::CredentialsManage => "credentials:manage",
            Self::DataRead => "data:read",
            Self::DataDelete => "data:delete",
        }
    }
}

/// Organization membership role resolved on every session authentication.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Full organization authority.
    Owner,
    /// Full delegated administration authority.
    Admin,
    /// Integration and credential management without configuration writes.
    Developer,
    /// Analytical data access.
    Analyst,
    /// Configuration metadata read access only.
    Viewer,
}

impl Role {
    /// Parses the database role value without accepting unknown privileges.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "developer" => Some(Self::Developer),
            "analyst" => Some(Self::Analyst),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    /// Tests the least-privilege role matrix.
    #[must_use]
    pub const fn allows(self, capability: Capability) -> bool {
        match self {
            Self::Owner | Self::Admin => true,
            Self::Developer => matches!(
                capability,
                Capability::ControlRead | Capability::CredentialsManage | Capability::DataRead
            ),
            Self::Analyst => matches!(capability, Capability::ControlRead | Capability::DataRead),
            Self::Viewer => matches!(capability, Capability::ControlRead),
        }
    }

    /// Returns capabilities in stable API order.
    #[must_use]
    pub fn capabilities(self) -> Vec<Capability> {
        [
            Capability::ControlRead,
            Capability::ControlWrite,
            Capability::CredentialsManage,
            Capability::DataRead,
            Capability::DataDelete,
        ]
        .into_iter()
        .filter(|capability| self.allows(*capability))
        .collect()
    }
}

/// A live user session whose role was resolved from current database state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedUserSession {
    /// Session identifier.
    pub session_id: String,
    /// Trusted tenant identifier.
    pub organization_id: String,
    /// Current account identifier.
    pub user_id: String,
    /// Current membership role.
    pub role: Role,
    /// Immutable session expiry.
    pub expires_at: time::OffsetDateTime,
}

/// An active SDK key with trusted tenant scope resolved by the server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedSDKKey {
    /// Key ID.
    pub key_id: String,
    /// Trusted organization ID.
    pub organization_id: String,
    /// Trusted project ID.
    pub project_id: String,
    /// Trusted environment ID.
    pub environment_id: String,
    /// Trusted data-source ID.
    pub data_source_id: String,
    /// Ingestion scopes granted to this key.
    pub scopes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_matrix_is_least_privilege() {
        for role in [Role::Owner, Role::Admin] {
            assert_eq!(role.capabilities().len(), 5);
        }
        assert!(!Role::Developer.allows(Capability::ControlWrite));
        assert!(!Role::Developer.allows(Capability::DataDelete));
        assert!(Role::Analyst.allows(Capability::DataRead));
        assert!(!Role::Analyst.allows(Capability::CredentialsManage));
        assert!(Role::Viewer.allows(Capability::ControlRead));
        assert!(!Role::Viewer.allows(Capability::DataRead));
        assert!(Role::parse("made-up").is_none());
    }
}
