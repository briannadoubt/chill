use std::{sync::LazyLock, time::Duration};

use regex::Regex;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{Capability, ControlPlaneError, CredentialKind, Store};

const MINIMUM_SESSION_LIFETIME: Duration = Duration::from_mins(5);
const MAXIMUM_SESSION_LIFETIME: Duration = Duration::from_hours(30 * 24);

static IDENTITY_ISSUER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z][a-z0-9_.-]{1,62}[a-z0-9]$")
        .unwrap_or_else(|error| unreachable!("static identity-issuer regex is invalid: {error}"))
});

/// Identity assertion already verified by a trusted provider adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedIdentity {
    /// Stable provider namespace.
    pub issuer: String,
    /// Stable provider subject.
    pub subject: String,
}

/// Newly issued human session returned exactly once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedUserSession {
    /// New session ID.
    pub session_id: String,
    /// Tenant ID.
    pub organization_id: String,
    /// User ID.
    pub user_id: String,
    /// One-time bearer credential.
    pub credential: String,
    /// Immutable expiry.
    pub expires_at: OffsetDateTime,
}

/// Request for a project/environment-scoped server credential.
#[derive(Clone, Debug)]
pub struct ServiceCredentialRequest {
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// Human-readable name.
    pub name: String,
    /// Least-privilege capabilities.
    pub scopes: Vec<Capability>,
    /// Optional immutable expiry.
    pub expires_at: Option<OffsetDateTime>,
}

/// Newly issued server credential returned exactly once.
#[derive(Clone, Debug)]
pub struct IssuedServiceCredential {
    /// New credential ID.
    pub credential_id: String,
    /// Tenant ID.
    pub organization_id: String,
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// One-time bearer credential.
    pub credential: String,
    /// Granted capabilities.
    pub scopes: Vec<Capability>,
    /// Optional immutable expiry.
    pub expires_at: Option<OffsetDateTime>,
}

/// Authenticated server credential with trusted scope.
#[derive(Clone, Debug)]
pub struct AuthenticatedServiceCredential {
    /// Credential ID.
    pub credential_id: String,
    /// Tenant ID.
    pub organization_id: String,
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// Granted capabilities.
    pub scopes: Vec<Capability>,
    /// Optional immutable expiry.
    pub expires_at: Option<OffsetDateTime>,
}

impl AuthenticatedServiceCredential {
    /// Tests one capability without widening the stored scope.
    #[must_use]
    pub fn allows(&self, capability: Capability) -> bool {
        self.scopes.contains(&capability)
    }
}

impl Store {
    /// Issues a user session when a verified identity belongs to exactly one active tenant.
    ///
    /// This is intended for identity providers that authenticate a person without presenting a
    /// tenant selector. Ambiguous identities are rejected rather than choosing a tenant.
    ///
    /// # Errors
    ///
    /// Returns validation, authentication, credential, RLS, or database errors.
    pub async fn issue_user_session_for_verified_identity(
        &self,
        identity: &VerifiedIdentity,
        lifetime: Duration,
    ) -> Result<IssuedUserSession, ControlPlaneError> {
        if !IDENTITY_ISSUER.is_match(&identity.issuer)
            || identity.subject.is_empty()
            || identity.subject.len() > 512
        {
            return Err(ControlPlaneError::InvalidInput(
                "verified identity is invalid".to_owned(),
            ));
        }
        let organizations = sqlx::query_scalar::<_, String>(
            "SELECT organization_id::text FROM control.lookup_identity_organizations($1, $2)",
        )
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .fetch_all(&self.pool)
        .await?;
        let [organization_id] = organizations.as_slice() else {
            return Err(ControlPlaneError::Unauthorized);
        };
        self.issue_user_session_after_identity_verification(organization_id, identity, lifetime)
            .await
    }

    /// Issues a user session after an external adapter has verified an identity assertion.
    ///
    /// # Errors
    ///
    /// Returns validation, authentication, credential, RLS, or database errors.
    pub async fn issue_user_session_after_identity_verification(
        &self,
        organization_id: &str,
        identity: &VerifiedIdentity,
        lifetime: Duration,
    ) -> Result<IssuedUserSession, ControlPlaneError> {
        validate_uuid("organization", organization_id)?;
        if !IDENTITY_ISSUER.is_match(&identity.issuer)
            || identity.subject.is_empty()
            || identity.subject.len() > 512
            || !(MINIMUM_SESSION_LIFETIME..=MAXIMUM_SESSION_LIFETIME).contains(&lifetime)
        {
            return Err(ControlPlaneError::InvalidInput(
                "verified identity or session lifetime is invalid".to_owned(),
            ));
        }
        let user_id = sqlx::query_scalar::<_, String>(
            "SELECT user_id::text FROM control.lookup_user_identity($1, $2, $3::uuid)",
        )
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .bind(organization_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ControlPlaneError::Unauthorized)?;
        let credential = self.issuer.issue(CredentialKind::UserSession)?;
        let duration = time::Duration::try_from(lifetime).map_err(|_| {
            ControlPlaneError::InvalidInput("session lifetime is invalid".to_owned())
        })?;
        let expires_at = OffsetDateTime::now_utc()
            .checked_add(duration)
            .ok_or_else(|| {
                ControlPlaneError::InvalidInput("session expiry is invalid".to_owned())
            })?;
        let mut transaction = self.begin_tenant(organization_id).await?;
        let (session_id, stored_expires_at) = sqlx::query_as::<_, (String, OffsetDateTime)>(
            r"
            INSERT INTO control.user_sessions (
                organization_id, user_id, prefix, secret_digest, expires_at
            )
            SELECT $1::uuid, membership.user_id, $3, $4, $5
            FROM control.organization_memberships AS membership
            JOIN control.users AS account ON account.id = membership.user_id
            JOIN control.organizations AS organization
              ON organization.id = membership.organization_id
            WHERE membership.organization_id = $1::uuid
              AND membership.user_id = $2::uuid AND membership.status = 'active'
              AND account.status = 'active' AND organization.status = 'active'
            RETURNING id::text, expires_at
        ",
        )
        .bind(organization_id)
        .bind(&user_id)
        .bind(&credential.prefix)
        .bind(credential.digest.as_slice())
        .bind(expires_at)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ControlPlaneError::Unauthorized)?;
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'auth.session.created', 'user_session',
                $3::uuid, 'identity_verified', jsonb_build_object(
                    'issuer', $4::text, 'expires_at', $5::text))
        ",
        )
        .bind(organization_id)
        .bind(&user_id)
        .bind(&session_id)
        .bind(&identity.issuer)
        .bind(stored_expires_at.to_string())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(IssuedUserSession {
            session_id,
            organization_id: organization_id.to_owned(),
            user_id,
            credential: credential.raw,
            expires_at: stored_expires_at,
        })
    }

    /// Revokes the currently authenticated user session.
    ///
    /// # Errors
    ///
    /// Returns authentication, RLS, or database errors.
    pub async fn revoke_user_session(&self, raw: &str) -> Result<(), ControlPlaneError> {
        let session = self.authenticate_user_session(raw).await?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let updated = sqlx::query(
            r"
            UPDATE control.user_sessions SET status = 'revoked', revoked_at = clock_timestamp()
            WHERE id = $1::uuid AND user_id = $2::uuid AND status = 'active'
        ",
        )
        .bind(&session.session_id)
        .bind(&session.user_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ControlPlaneError::Unauthorized);
        }
        audit_session(&mut transaction, &session, "auth.session.revoked", None).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Atomically rotates a user session while preserving its original expiry.
    ///
    /// # Errors
    ///
    /// Returns authentication, credential, RLS, or database errors.
    pub async fn rotate_user_session(
        &self,
        raw: &str,
    ) -> Result<IssuedUserSession, ControlPlaneError> {
        let session = self.authenticate_user_session(raw).await?;
        let issued = self.issuer.issue(CredentialKind::UserSession)?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let expires_at = sqlx::query_scalar::<_, OffsetDateTime>(
            r"
            SELECT expires_at FROM control.user_sessions
            WHERE id = $1::uuid AND user_id = $2::uuid AND status = 'active'
              AND expires_at > statement_timestamp() FOR UPDATE
        ",
        )
        .bind(&session.session_id)
        .bind(&session.user_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ControlPlaneError::Unauthorized)?;
        let replacement_id = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.user_sessions (
                organization_id, user_id, prefix, secret_digest, expires_at, rotated_from_id
            ) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6::uuid)
            RETURNING id::text
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(&issued.prefix)
        .bind(issued.digest.as_slice())
        .bind(expires_at)
        .bind(&session.session_id)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            UPDATE control.user_sessions
            SET status = 'revoked', revoked_at = clock_timestamp(), replaced_by_id = $2::uuid
            WHERE id = $1::uuid AND status = 'active'
        ",
        )
        .bind(&session.session_id)
        .bind(&replacement_id)
        .execute(&mut *transaction)
        .await?;
        audit_session(
            &mut transaction,
            &session,
            "auth.session.rotated",
            Some(&replacement_id),
        )
        .await?;
        transaction.commit().await?;
        Ok(IssuedUserSession {
            session_id: replacement_id,
            organization_id: session.organization_id,
            user_id: session.user_id,
            credential: issued.raw,
            expires_at,
        })
    }

    /// Issues a least-privilege service credential authorized by the current user role.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, credential, RLS, or database errors.
    pub async fn issue_service_credential(
        &self,
        raw_session: &str,
        mut request: ServiceCredentialRequest,
    ) -> Result<IssuedServiceCredential, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::CredentialsManage)
            .await?;
        validate_service_request(&mut request)?;
        if request
            .scopes
            .iter()
            .any(|scope| !session.role.allows(*scope))
        {
            return Err(ControlPlaneError::Forbidden);
        }
        let issued = self.issuer.issue(CredentialKind::Service)?;
        let scope_values = capability_values(&request.scopes);
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let credential_id = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.service_credentials (
                organization_id, project_id, environment_id, name, prefix,
                secret_digest, scopes, created_by, expires_at
            )
            SELECT $1::uuid, project.id, environment.id, $4, $5, $6, $7,
                $8::uuid, $9
            FROM control.projects AS project
            JOIN control.environments AS environment
              ON environment.project_id = project.id
             AND environment.organization_id = project.organization_id
            WHERE project.organization_id = $1::uuid AND project.id = $2::uuid
              AND environment.id = $3::uuid AND project.status = 'active'
              AND environment.status = 'active'
            RETURNING id::text
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .bind(&request.name)
        .bind(&issued.prefix)
        .bind(issued.digest.as_slice())
        .bind(&scope_values)
        .bind(&session.user_id)
        .bind(request.expires_at)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ControlPlaneError::Forbidden)?;
        audit_service(
            &mut transaction,
            &session.organization_id,
            &session.user_id,
            "credential.service.created",
            &credential_id,
            Some((&request.project_id, &request.environment_id, &scope_values)),
            None,
        )
        .await?;
        transaction.commit().await?;
        Ok(IssuedServiceCredential {
            credential_id,
            organization_id: session.organization_id,
            project_id: request.project_id,
            environment_id: request.environment_id,
            credential: issued.raw,
            scopes: request.scopes,
            expires_at: request.expires_at,
        })
    }

    /// Authenticates an active service credential and resolves its current scope.
    ///
    /// # Errors
    ///
    /// Returns unauthorized for malformed, revoked, expired, or unknown credentials.
    pub async fn authenticate_service_credential(
        &self,
        raw: &str,
    ) -> Result<AuthenticatedServiceCredential, ControlPlaneError> {
        let prefix = crate::parse_credential_prefix(raw, CredentialKind::Service)
            .map_err(|_| ControlPlaneError::Unauthorized)?;
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                Vec<u8>,
                Vec<String>,
                Option<OffsetDateTime>,
            ),
        >(
            r"
            SELECT credential_id::text, organization_id::text, project_id::text,
                environment_id::text, secret_digest, scopes, expires_at
            FROM control.lookup_service_credential($1)
        ",
        )
        .bind(prefix)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ControlPlaneError::Unauthorized)?;
        if !self.issuer.verify(raw, &row.4) {
            return Err(ControlPlaneError::Unauthorized);
        }
        let scopes = row
            .5
            .iter()
            .map(|scope| Capability::parse(scope).ok_or(ControlPlaneError::Unauthorized))
            .collect::<Result<Vec<_>, _>>()?;
        let credential = AuthenticatedServiceCredential {
            credential_id: row.0,
            organization_id: row.1,
            project_id: row.2,
            environment_id: row.3,
            scopes,
            expires_at: row.6,
        };
        let mut transaction = self.begin_tenant(&credential.organization_id).await?;
        sqlx::query("SELECT control.mark_service_credential_used($1::uuid)")
            .bind(&credential.credential_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(credential)
    }

    /// Rotates an active service credential while preserving scope and expiry.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, credential, RLS, or database errors.
    #[allow(
        clippy::too_many_lines,
        reason = "credential locking, authorization, replacement, revocation, and audit are atomic"
    )]
    pub async fn rotate_service_credential(
        &self,
        raw_session: &str,
        credential_id: &str,
    ) -> Result<IssuedServiceCredential, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::CredentialsManage)
            .await?;
        validate_uuid("service credential", credential_id)?;
        let issued = self.issuer.issue(CredentialKind::Service)?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let row =
            sqlx::query_as::<_, (String, String, String, Vec<String>, Option<OffsetDateTime>)>(
                r"
            SELECT project_id::text, environment_id::text, name, scopes, expires_at
            FROM control.service_credentials WHERE id = $1::uuid AND status = 'active'
              AND (expires_at IS NULL OR expires_at > statement_timestamp()) FOR UPDATE
        ",
            )
            .bind(credential_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(ControlPlaneError::Forbidden)?;
        let scopes = row
            .3
            .iter()
            .map(|scope| Capability::parse(scope).ok_or(ControlPlaneError::Forbidden))
            .collect::<Result<Vec<_>, _>>()?;
        if scopes.iter().any(|scope| !session.role.allows(*scope)) {
            return Err(ControlPlaneError::Forbidden);
        }
        let replacement_id = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.service_credentials (
                organization_id, project_id, environment_id, name, prefix,
                secret_digest, scopes, created_by, expires_at, rotated_from_id
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7,
                $8::uuid, $9, $10::uuid) RETURNING id::text
        ",
        )
        .bind(&session.organization_id)
        .bind(&row.0)
        .bind(&row.1)
        .bind(&row.2)
        .bind(&issued.prefix)
        .bind(issued.digest.as_slice())
        .bind(&row.3)
        .bind(&session.user_id)
        .bind(row.4)
        .bind(credential_id)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            UPDATE control.service_credentials
            SET status = 'revoked', revoked_at = clock_timestamp(), replaced_by_id = $2::uuid
            WHERE id = $1::uuid AND status = 'active'
        ",
        )
        .bind(credential_id)
        .bind(&replacement_id)
        .execute(&mut *transaction)
        .await?;
        audit_service(
            &mut transaction,
            &session.organization_id,
            &session.user_id,
            "credential.service.rotated",
            credential_id,
            None,
            Some(&replacement_id),
        )
        .await?;
        transaction.commit().await?;
        Ok(IssuedServiceCredential {
            credential_id: replacement_id,
            organization_id: session.organization_id,
            project_id: row.0,
            environment_id: row.1,
            credential: issued.raw,
            scopes,
            expires_at: row.4,
        })
    }

    /// Revokes one active service credential in the current tenant.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, RLS, or database errors.
    pub async fn revoke_service_credential(
        &self,
        raw_session: &str,
        credential_id: &str,
    ) -> Result<(), ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::CredentialsManage)
            .await?;
        validate_uuid("service credential", credential_id)?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let updated = sqlx::query(
            r"
            UPDATE control.service_credentials SET status = 'revoked', revoked_at = clock_timestamp()
            WHERE id = $1::uuid AND status = 'active'
        ",
        )
        .bind(credential_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ControlPlaneError::Forbidden);
        }
        audit_service(
            &mut transaction,
            &session.organization_id,
            &session.user_id,
            "credential.service.revoked",
            credential_id,
            None,
            None,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn validate_service_request(
    request: &mut ServiceCredentialRequest,
) -> Result<(), ControlPlaneError> {
    validate_uuid("project", &request.project_id)?;
    validate_uuid("environment", &request.environment_id)?;
    request.name = request.name.trim().to_owned();
    request.scopes.sort_unstable();
    request.scopes.dedup();
    if request.name.chars().count() == 0
        || request.name.chars().count() > 160
        || request.scopes.is_empty()
        || request.scopes.len() > 8
        || request
            .expires_at
            .is_some_and(|expiry| expiry <= OffsetDateTime::now_utc() + time::Duration::minutes(1))
    {
        return Err(ControlPlaneError::InvalidInput(
            "service credential request is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn validate_uuid(label: &str, value: &str) -> Result<(), ControlPlaneError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ControlPlaneError::InvalidInput(format!("{label} ID is invalid")))
}

fn capability_values(capabilities: &[Capability]) -> Vec<&'static str> {
    capabilities.iter().map(|scope| scope.as_str()).collect()
}

async fn audit_session(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session: &crate::AuthenticatedUserSession,
    action: &str,
    replacement_id: Option<&str>,
) -> Result<(), ControlPlaneError> {
    sqlx::query(
        r"
        INSERT INTO control.audit_log (
            organization_id, actor_user_id, action, target_type, target_id,
            reason_code, details
        ) VALUES ($1::uuid, $2::uuid, $3, 'user_session', $4::uuid,
            'user_requested', CASE WHEN $5::text IS NULL THEN '{}'::jsonb
                ELSE jsonb_build_object('replacement_id', $5::text) END)
    ",
    )
    .bind(&session.organization_id)
    .bind(&session.user_id)
    .bind(action)
    .bind(&session.session_id)
    .bind(replacement_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn audit_service(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: &str,
    user_id: &str,
    action: &str,
    credential_id: &str,
    created: Option<(&str, &str, &[&str])>,
    replacement_id: Option<&str>,
) -> Result<(), ControlPlaneError> {
    let (project_id, environment_id, scopes) = created.map_or((None, None, None), |value| {
        (Some(value.0), Some(value.1), Some(value.2))
    });
    sqlx::query(
        r"
        INSERT INTO control.audit_log (
            organization_id, actor_user_id, action, target_type, target_id,
            reason_code, details
        ) VALUES ($1::uuid, $2::uuid, $3, 'service_credential', $4::uuid,
            'user_requested', CASE
                WHEN $5::text IS NOT NULL THEN jsonb_build_object(
                    'project_id', $5::text, 'environment_id', $6::text, 'scopes', $7::text[])
                WHEN $8::text IS NOT NULL THEN jsonb_build_object('replacement_id', $8::text)
                ELSE '{}'::jsonb END)
    ",
    )
    .bind(organization_id)
    .bind(user_id)
    .bind(action)
    .bind(credential_id)
    .bind(project_id)
    .bind(environment_id)
    .bind(scopes)
    .bind(replacement_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}
