use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{Capability, ControlPlaneError, Store, privacy::policy_version_is_valid};

const CAPTURE_CLASSES: &[&str] = &["essential", "analytics", "diagnostic", "replay"];

/// Effective source-side collection controls for one SDK key.
#[derive(Debug, Serialize)]
pub struct CollectionPolicy {
    /// Monotonic environment policy revision.
    pub revision: u64,
    /// Referenced active privacy-policy version.
    pub policy_version: String,
    /// Whether collection is enabled.
    pub enabled: bool,
    /// Capture classes disabled at the source.
    pub disabled_capture_classes: Vec<String>,
    /// Effective timestamp expressed as Unix nanoseconds.
    pub effective_at_unix_nano: u64,
}

/// Optimistic, idempotent request to replace an active collection policy.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetCollectionPolicyRequest {
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// Revision that must currently be active.
    pub expected_revision: u64,
    /// Active privacy-policy version referenced by the replacement.
    pub policy_version: String,
    /// Whether collection is enabled.
    pub enabled: bool,
    /// Capture classes disabled at the source.
    pub disabled_capture_classes: Vec<String>,
    /// Tenant-unique request key.
    pub idempotency_key: String,
}

impl Store {
    /// Reads the current collection policy for an authenticated SDK key.
    ///
    /// # Errors
    ///
    /// Returns unauthorized for an invalid key, forbidden when no policy is
    /// available, or a database error when policy state cannot be read.
    pub async fn current_collection_policy_for_sdk_key(
        &self,
        raw: &str,
    ) -> Result<CollectionPolicy, ControlPlaneError> {
        let key = self.authenticate_sdk_key(raw).await?;
        let mut transaction = self.begin_tenant(&key.organization_id).await?;
        let row = sqlx::query_as::<_, (i64, String, bool, Vec<String>, OffsetDateTime)>(
            r"
            SELECT revision, policy_version, enabled, disabled_capture_classes, effective_at
            FROM control.collection_policies
            WHERE organization_id = $1::uuid AND project_id = $2::uuid
              AND environment_id = $3::uuid AND status = 'active'
              AND effective_at <= statement_timestamp()
            ORDER BY revision DESC LIMIT 1
        ",
        )
        .bind(&key.organization_id)
        .bind(&key.project_id)
        .bind(&key.environment_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ControlPlaneError::Forbidden)?;
        transaction.commit().await?;
        let revision = u64::try_from(row.0).map_err(|_| {
            ControlPlaneError::InvalidInput("collection policy revision is invalid".to_owned())
        })?;
        let effective_at_unix_nano = u64::try_from(row.4.unix_timestamp_nanos()).map_err(|_| {
            ControlPlaneError::InvalidInput(
                "collection policy effective timestamp is invalid".to_owned(),
            )
        })?;
        Ok(CollectionPolicy {
            revision,
            policy_version: row.1,
            enabled: row.2,
            disabled_capture_classes: row.3,
            effective_at_unix_nano,
        })
    }

    /// Replaces an active collection policy with optimistic concurrency and idempotency.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, conflict, RLS, or database errors.
    #[allow(
        clippy::too_many_lines,
        reason = "locking, idempotency, optimistic concurrency, activation, and audit are one transaction"
    )]
    pub async fn set_collection_policy(
        &self,
        raw_session: &str,
        mut request: SetCollectionPolicyRequest,
    ) -> Result<CollectionPolicy, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlWrite)
            .await?;
        validate_collection_request(&mut request)?;
        let expected_revision = i64::try_from(request.expected_revision).map_err(|_| {
            ControlPlaneError::InvalidInput("expected collection revision is invalid".to_owned())
        })?;
        let next_revision = expected_revision.checked_add(1).ok_or_else(|| {
            ControlPlaneError::InvalidInput("expected collection revision is invalid".to_owned())
        })?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1381258073))")
            .bind(&request.environment_id)
            .execute(&mut *transaction)
            .await?;
        let existing = sqlx::query_as::<
            _,
            (
                String,
                String,
                i64,
                String,
                bool,
                Vec<String>,
                OffsetDateTime,
            ),
        >(
            r"
            SELECT project_id::text, environment_id::text, revision, policy_version,
                enabled, disabled_capture_classes, effective_at
            FROM control.collection_policies
            WHERE organization_id = $1::uuid AND idempotency_key = $2
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(row) = existing {
            if row.0 != request.project_id
                || row.1 != request.environment_id
                || row.2 != next_revision
                || row.3 != request.policy_version
                || row.4 != request.enabled
                || row.5 != request.disabled_capture_classes
            {
                return Err(ControlPlaneError::Conflict);
            }
            transaction.commit().await?;
            return collection_policy_from_row(row.2, row.3, row.4, row.5, row.6);
        }
        let current = sqlx::query_as::<_, (String, i64)>(
            r"
            SELECT id::text, revision FROM control.collection_policies
            WHERE organization_id = $1::uuid AND project_id = $2::uuid
              AND environment_id = $3::uuid AND status = 'active'
            FOR UPDATE
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ControlPlaneError::Forbidden)?;
        if current.1 != expected_revision {
            return Err(ControlPlaneError::Conflict);
        }
        let active_policy_version = sqlx::query_scalar::<_, String>(
            r"
            SELECT document->>'policy_version' FROM control.privacy_policies
            WHERE organization_id = $1::uuid AND project_id = $2::uuid
              AND environment_id = $3::uuid AND status = 'active'
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ControlPlaneError::Forbidden)?;
        if active_policy_version != request.policy_version {
            return Err(ControlPlaneError::InvalidInput(
                "collection policy must reference the active privacy policy".to_owned(),
            ));
        }
        sqlx::query(
            "UPDATE control.collection_policies SET status = 'retired' WHERE id = $1::uuid",
        )
        .bind(&current.0)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query_as::<_, (i64, String, bool, Vec<String>, OffsetDateTime)>(
            r"
            INSERT INTO control.collection_policies (
                organization_id, project_id, environment_id, revision, policy_version,
                enabled, disabled_capture_classes, created_by, idempotency_key
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7, $8::uuid, $9)
            RETURNING revision, policy_version, enabled, disabled_capture_classes, effective_at
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .bind(next_revision)
        .bind(&request.policy_version)
        .bind(request.enabled)
        .bind(&request.disabled_capture_classes)
        .bind(&session.user_id)
        .bind(&request.idempotency_key)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                request_id, reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'privacy.collection_policy.changed', 'environment',
                $3::uuid, $4, 'privacy.policy_change', jsonb_build_object(
                    'revision', $5::bigint, 'policy_version', $6::text,
                    'enabled', $7::boolean, 'disabled_capture_classes', $8::text[]))
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(&request.environment_id)
        .bind(&request.idempotency_key)
        .bind(row.0)
        .bind(&row.1)
        .bind(row.2)
        .bind(&row.3)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        collection_policy_from_row(row.0, row.1, row.2, row.3, row.4)
    }
}

fn validate_collection_request(
    request: &mut SetCollectionPolicyRequest,
) -> Result<(), ControlPlaneError> {
    if uuid::Uuid::parse_str(&request.project_id).is_err()
        || uuid::Uuid::parse_str(&request.environment_id).is_err()
        || request.expected_revision < 1
        || !policy_version_is_valid(&request.policy_version)
    {
        return Err(ControlPlaneError::InvalidInput(
            "collection policy identity or version is invalid".to_owned(),
        ));
    }
    request.disabled_capture_classes.sort_unstable();
    request.disabled_capture_classes.dedup();
    if request.disabled_capture_classes.len() > 4
        || request
            .disabled_capture_classes
            .iter()
            .any(|class| !CAPTURE_CLASSES.contains(&class.as_str()))
    {
        return Err(ControlPlaneError::InvalidInput(
            "collection policy capture classes are invalid".to_owned(),
        ));
    }
    request.idempotency_key = request.idempotency_key.trim().to_owned();
    if request.idempotency_key.is_empty() || request.idempotency_key.len() > 128 {
        return Err(ControlPlaneError::InvalidInput(
            "collection policy idempotency key is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn collection_policy_from_row(
    revision: i64,
    policy_version: String,
    enabled: bool,
    disabled_capture_classes: Vec<String>,
    effective_at: OffsetDateTime,
) -> Result<CollectionPolicy, ControlPlaneError> {
    Ok(CollectionPolicy {
        revision: u64::try_from(revision).map_err(|_| {
            ControlPlaneError::InvalidInput("collection policy revision is invalid".to_owned())
        })?,
        policy_version,
        enabled,
        disabled_capture_classes,
        effective_at_unix_nano: u64::try_from(effective_at.unix_timestamp_nanos()).map_err(
            |_| {
                ControlPlaneError::InvalidInput(
                    "collection policy effective timestamp is invalid".to_owned(),
                )
            },
        )?,
    })
}

/// Builds the SDK-authenticated source collection-state endpoint.
pub fn collection_router(store: Store) -> Router {
    Router::new()
        .route("/v1/chill/collection-state", get(collection_state))
        .layer(middleware::from_fn(collection_headers))
        .with_state(store)
}

async fn collection_state(State(store): State<Store>, headers: HeaderMap) -> Response {
    let Ok(credential) = sdk_credential(&headers) else {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    };
    match store
        .current_collection_policy_for_sdk_key(&credential)
        .await
    {
        Ok(policy) => {
            let etag = HeaderValue::try_from(format!("\"collection-{}\"", policy.revision));
            let mut response = Json(policy).into_response();
            match etag {
                Ok(value) => {
                    response.headers_mut().insert("etag", value);
                    response
                }
                Err(_) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "collection policy unavailable",
                )
                    .into_response(),
            }
        }
        Err(ControlPlaneError::Unauthorized) => {
            (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
        }
        Err(ControlPlaneError::Forbidden) => {
            (StatusCode::FORBIDDEN, "collection unavailable").into_response()
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "collection policy unavailable",
        )
            .into_response(),
    }
}

fn sdk_credential(headers: &HeaderMap) -> Result<String, ()> {
    let mut credentials = Vec::new();
    for value in headers.get_all(AUTHORIZATION) {
        let value = value.to_str().map_err(|_| ())?;
        let credential = value.strip_prefix("Bearer ").ok_or(())?;
        if credential.is_empty() {
            return Err(());
        }
        credentials.push(credential.to_owned());
    }
    for value in headers.get_all("x-chill-sdk-key") {
        let credential = value.to_str().map_err(|_| ())?;
        if credential.is_empty() {
            return Err(());
        }
        credentials.push(credential.to_owned());
    }
    if credentials.len() == 1 {
        credentials.pop().ok_or(())
    } else {
        Err(())
    }
}

async fn collection_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_credential_requires_exactly_one_supported_header() {
        let mut headers = HeaderMap::new();
        assert!(sdk_credential(&headers).is_err());
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer ch_sk_one"));
        assert_eq!(sdk_credential(&headers).ok().as_deref(), Some("ch_sk_one"));
        headers.insert("x-chill-sdk-key", HeaderValue::from_static("ch_sk_two"));
        assert!(sdk_credential(&headers).is_err());
    }
}
