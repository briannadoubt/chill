use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    AuthenticatedSDKKey, AuthenticatedUserSession, Capability, CredentialError, CredentialIssuer,
    CredentialKind, Role, parse_credential_prefix,
};

/// Fail-closed errors shared by control-plane transports.
#[derive(Debug, Error)]
pub enum ControlPlaneError {
    /// Authentication failed without disclosing which check failed.
    #[error("unauthorized")]
    Unauthorized,
    /// The authenticated principal lacks the requested capability.
    #[error("forbidden")]
    Forbidden,
    /// A unique or idempotency contract rejected a duplicate mutation.
    #[error("conflict")]
    Conflict,
    /// A caller supplied malformed input.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A database operation failed.
    #[error("control-plane database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// Cryptographic credential issuance failed.
    #[error("credential operation failed: {0}")]
    Credential(#[from] CredentialError),
}

/// PostgreSQL-backed control plane with a shared credential issuer.
#[derive(Clone)]
pub struct Store {
    pub(crate) pool: PgPool,
    pub(crate) issuer: CredentialIssuer,
}

impl Store {
    /// Creates a store over an already bounded connection pool.
    #[must_use]
    pub const fn new(pool: PgPool, issuer: CredentialIssuer) -> Self {
        Self { pool, issuer }
    }

    /// Begins a transaction with the trusted tenant context installed for RLS.
    ///
    /// # Errors
    ///
    /// Returns an input error for a non-UUID organization or a database error.
    pub async fn begin_tenant(
        &self,
        organization_id: &str,
    ) -> Result<Transaction<'_, Postgres>, ControlPlaneError> {
        Uuid::parse_str(organization_id).map_err(|_| {
            ControlPlaneError::InvalidInput("organization must be a UUID".to_owned())
        })?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.organization_id', $1, true)")
            .bind(organization_id)
            .execute(&mut *transaction)
            .await?;
        Ok(transaction)
    }

    /// Authenticates a user session and resolves the current organization role.
    ///
    /// # Errors
    ///
    /// Returns unauthorized for malformed, expired, revoked, unknown, or mismatched sessions.
    pub async fn authenticate_user_session(
        &self,
        raw: &str,
    ) -> Result<AuthenticatedUserSession, ControlPlaneError> {
        let prefix = parse_credential_prefix(raw, CredentialKind::UserSession)
            .map_err(|_| ControlPlaneError::Unauthorized)?;
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                Vec<u8>,
                time::OffsetDateTime,
            ),
        >(
            r"
            SELECT session_id::text, organization_id::text, user_id::text, role,
                   secret_digest, expires_at
            FROM control.lookup_user_session($1)
        ",
        )
        .bind(prefix)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ControlPlaneError::Unauthorized)?;
        if !self.issuer.verify(raw, &row.4) {
            return Err(ControlPlaneError::Unauthorized);
        }
        let role = Role::parse(&row.3).ok_or(ControlPlaneError::Unauthorized)?;
        let session = AuthenticatedUserSession {
            session_id: row.0,
            organization_id: row.1,
            user_id: row.2,
            role,
            expires_at: row.5,
        };
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        sqlx::query("SELECT control.mark_user_session_used($1::uuid)")
            .bind(&session.session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(session)
    }

    /// Authenticates a session and enforces one current role capability.
    ///
    /// # Errors
    ///
    /// Returns authentication errors or forbidden when the current role lacks access.
    pub async fn require_user_capability(
        &self,
        raw: &str,
        capability: Capability,
    ) -> Result<AuthenticatedUserSession, ControlPlaneError> {
        let session = self.authenticate_user_session(raw).await?;
        if !session.role.allows(capability) {
            return Err(ControlPlaneError::Forbidden);
        }
        Ok(session)
    }

    /// Authenticates an SDK key and verifies every bound resource remains active.
    ///
    /// # Errors
    ///
    /// Returns unauthorized for malformed, revoked, expired, digest-mismatched,
    /// or inactive-scope keys, and database errors for unavailable state.
    pub async fn authenticate_sdk_key(
        &self,
        raw: &str,
    ) -> Result<AuthenticatedSDKKey, ControlPlaneError> {
        let prefix = parse_credential_prefix(raw, CredentialKind::SdkKey)
            .map_err(|_| ControlPlaneError::Unauthorized)?;
        let row =
            sqlx::query_as::<_, (String, String, String, String, String, Vec<u8>, Vec<String>)>(
                r"
            SELECT key_id::text, organization_id::text, project_id::text,
                   environment_id::text, data_source_id::text, secret_digest, scopes
            FROM control.lookup_sdk_key($1)
        ",
            )
            .bind(prefix)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(ControlPlaneError::Unauthorized)?;
        if !self.issuer.verify(raw, &row.5) {
            return Err(ControlPlaneError::Unauthorized);
        }
        let key = AuthenticatedSDKKey {
            key_id: row.0,
            organization_id: row.1,
            project_id: row.2,
            environment_id: row.3,
            data_source_id: row.4,
            scopes: row.6,
        };
        let mut transaction = self.begin_tenant(&key.organization_id).await?;
        let active = sqlx::query_scalar::<_, bool>(r"
            SELECT EXISTS (
                SELECT 1 FROM control.organizations AS organization
                JOIN control.projects AS project ON project.organization_id = organization.id
                JOIN control.environments AS environment
                  ON environment.project_id = project.id AND environment.organization_id = organization.id
                JOIN control.data_sources AS source
                  ON source.environment_id = environment.id AND source.project_id = project.id
                 AND source.organization_id = organization.id
                WHERE organization.id = $1::uuid AND project.id = $2::uuid
                  AND environment.id = $3::uuid AND source.id = $4::uuid
                  AND organization.status = 'active' AND project.status = 'active'
                  AND environment.status = 'active' AND source.status = 'active'
            )
        ")
        .bind(&key.organization_id)
        .bind(&key.project_id)
        .bind(&key.environment_id)
        .bind(&key.data_source_id)
        .fetch_one(&mut *transaction)
        .await?;
        if !active {
            return Err(ControlPlaneError::Unauthorized);
        }
        sqlx::query("SELECT control.mark_sdk_key_used($1::uuid)")
            .bind(&key.key_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(key)
    }
}
