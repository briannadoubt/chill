use std::{sync::LazyLock, time::Duration};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::{Postgres, Transaction, types::Json};

use crate::{ControlPlaneError, CredentialKind, Store, parse_environment_privacy_policy};

static SLUG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z][a-z0-9-]{1,62}[a-z0-9]$")
        .unwrap_or_else(|error| unreachable!("static slug regex is invalid: {error}"))
});
static SEMANTIC_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
        .unwrap_or_else(|error| unreachable!("static semantic-version regex is invalid: {error}"))
});

/// Complete first-tenant bootstrap request.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapRequest {
    /// Globally unique bootstrap operation key.
    pub idempotency_key: String,
    /// Initial organization.
    pub organization: NamedSlug,
    /// Initial project.
    pub project: NamedSlug,
    /// Initial environment.
    pub environment: BootstrapEnvironment,
    /// Initial ingestion source.
    pub data_source: BootstrapDataSource,
    /// Initial owner account.
    pub owner: BootstrapOwner,
    /// Initial immutable behavior schema.
    pub schema: BootstrapSchema,
    /// Initial sampling policy.
    pub sampling: BootstrapSamplingPolicy,
    /// Initial privacy policy.
    pub privacy: BootstrapPrivacyPolicy,
    /// Initial resource limits.
    pub quota: BootstrapQuota,
    /// Initial SDK key request.
    pub sdk_key: BootstrapSDKKey,
}

/// Human-readable name with a stable URL-safe slug.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NamedSlug {
    /// Stable slug.
    pub slug: String,
    /// Display name.
    pub name: String,
}

/// Initial environment definition.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapEnvironment {
    /// Stable slug.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Environment class.
    pub kind: String,
    /// Behavior retention duration.
    pub retention_days: i32,
}

/// Initial data-source definition.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapDataSource {
    /// Display name.
    pub name: String,
    /// Platform class.
    pub kind: String,
}

/// Initial owner identity.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapOwner {
    /// Normalized account email.
    pub email: String,
    /// Display name.
    pub display_name: String,
}

/// Initial behavior-schema definition.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapSchema {
    /// Semantic version.
    pub version: String,
    /// Canonical schema URL.
    pub url: String,
    /// JSON Schema object.
    pub definition: Value,
    /// Compatibility mode.
    pub compatibility: String,
}

/// Initial sampling ratios.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapSamplingPolicy {
    /// Behavior numerator.
    pub behavior_numerator: i64,
    /// Behavior denominator.
    pub behavior_denominator: i64,
    /// Replay numerator.
    pub replay_numerator: i64,
    /// Replay denominator.
    pub replay_denominator: i64,
    /// Deterministic salt identifier.
    pub salt_version: String,
}

/// Initial privacy document wrapper.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapPrivacyPolicy {
    /// Complete fail-closed policy.
    pub document: Value,
}

/// Initial environment quotas.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapQuota {
    /// Request rate.
    pub requests_per_minute: i32,
    /// Record rate.
    pub records_per_minute: i32,
    /// Daily replay-byte allowance.
    pub replay_bytes_per_day: i64,
    /// Concurrent queries.
    pub query_concurrency: i32,
    /// Per-query scan-byte allowance.
    pub query_scan_bytes: i64,
}

/// Initial SDK key name and ingestion scopes.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapSDKKey {
    /// Display name.
    pub name: String,
    /// Ingestion scopes.
    pub scopes: Vec<String>,
}

/// Idempotent bootstrap receipt with a one-time SDK credential on creation.
#[derive(Clone, Debug, Default, Serialize)]
pub struct BootstrapResult {
    /// Whether this invocation created the tenant.
    pub created: bool,
    /// Organization ID.
    pub organization_id: String,
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// Data-source ID.
    pub data_source_id: String,
    /// SDK key ID.
    pub sdk_key_id: String,
    /// Owner user ID.
    pub owner_user_id: String,
    /// Raw SDK key, present only on initial creation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdk_key: Option<String>,
}

impl BootstrapRequest {
    /// Builds the checked local-development bootstrap contract.
    #[must_use]
    pub fn local_default(owner_email: &str, schema_definition: Value) -> Self {
        Self {
            idempotency_key: "chill-local-bootstrap-v1".to_owned(),
            organization: NamedSlug {
                slug: "chill-local".to_owned(),
                name: "Chill Local".to_owned(),
            },
            project: NamedSlug {
                slug: "chill".to_owned(),
                name: "Chill".to_owned(),
            },
            environment: BootstrapEnvironment {
                slug: "development".to_owned(),
                name: "Development".to_owned(),
                kind: "development".to_owned(),
                retention_days: 30,
            },
            data_source: BootstrapDataSource {
                name: "Apple SDK".to_owned(),
                kind: "apple".to_owned(),
            },
            owner: BootstrapOwner {
                email: owner_email.to_owned(),
                display_name: "Chill Owner".to_owned(),
            },
            schema: BootstrapSchema {
                version: "1.0.0".to_owned(),
                url: "https://schemas.chill.dev/behavior/v1/envelope.schema.json".to_owned(),
                definition: schema_definition,
                compatibility: "exact".to_owned(),
            },
            sampling: BootstrapSamplingPolicy {
                behavior_numerator: 1,
                behavior_denominator: 1,
                replay_numerator: 0,
                replay_denominator: 1,
                salt_version: "v1".to_owned(),
            },
            privacy: BootstrapPrivacyPolicy {
                document: serde_json::from_str(include_str!(
                    "../../../../policies/privacy/v1/default-environment-policy.json"
                ))
                .unwrap_or_else(|error| {
                    unreachable!("checked default privacy policy is invalid: {error}")
                }),
            },
            quota: BootstrapQuota {
                requests_per_minute: 600,
                records_per_minute: 60_000,
                replay_bytes_per_day: 1 << 30,
                query_concurrency: 2,
                query_scan_bytes: 1 << 30,
            },
            sdk_key: BootstrapSDKKey {
                name: "Local Apple SDK".to_owned(),
                scopes: vec!["ingest:otlp".to_owned(), "ingest:replay".to_owned()],
            },
        }
    }

    fn normalize_and_validate(&mut self) -> Result<String, ControlPlaneError> {
        self.owner.email = self.owner.email.trim().to_lowercase();
        self.sdk_key.scopes.sort_unstable();
        self.sdk_key.scopes.dedup();
        for (label, slug, name) in [
            (
                "organization",
                &self.organization.slug,
                &self.organization.name,
            ),
            ("project", &self.project.slug, &self.project.name),
            (
                "environment",
                &self.environment.slug,
                &self.environment.name,
            ),
        ] {
            if !SLUG.is_match(slug) || !(1..=160).contains(&name.chars().count()) {
                return invalid(format!("{label} name or slug is invalid"));
            }
        }
        if !(16..=128).contains(&self.idempotency_key.len())
            || !(1..=3650).contains(&self.environment.retention_days)
            || !["production", "staging", "development", "test"]
                .contains(&self.environment.kind.as_str())
            || !["apple", "android", "web", "server", "otlp"]
                .contains(&self.data_source.kind.as_str())
            || !(1..=160).contains(&self.data_source.name.chars().count())
            || !valid_email(&self.owner.email)
            || !(1..=160).contains(&self.owner.display_name.chars().count())
        {
            return invalid("bootstrap identity or environment is invalid");
        }
        if !SEMANTIC_VERSION.is_match(&self.schema.version)
            || self.schema.url.is_empty()
            || self.schema.url.len() > 2048
            || !["exact", "backward", "forward", "full"]
                .contains(&self.schema.compatibility.as_str())
            || !self.schema.definition.is_object()
        {
            return invalid("bootstrap schema is invalid");
        }
        let privacy = parse_environment_privacy_policy(self.privacy.document.clone())?;
        let sampling = &self.sampling;
        if sampling.behavior_denominator <= 0
            || sampling.behavior_numerator < 0
            || sampling.behavior_numerator > sampling.behavior_denominator
            || sampling.replay_denominator <= 0
            || sampling.replay_numerator < 0
            || sampling.replay_numerator > sampling.replay_denominator
            || sampling.salt_version.is_empty()
            || sampling.salt_version.len() > 64
        {
            return invalid("bootstrap sampling policy is invalid");
        }
        let quota = &self.quota;
        if quota.requests_per_minute < 1
            || quota.records_per_minute < 1
            || quota.query_concurrency < 1
            || quota.replay_bytes_per_day < 0
            || quota.query_scan_bytes < 1 << 20
            || !(1..=160).contains(&self.sdk_key.name.chars().count())
            || self.sdk_key.scopes.is_empty()
            || self.sdk_key.scopes.len() > 8
            || self
                .sdk_key
                .scopes
                .iter()
                .any(|scope| !["ingest:otlp", "ingest:replay"].contains(&scope.as_str()))
        {
            return invalid("bootstrap quota or SDK key is invalid");
        }
        Ok(privacy.policy_version)
    }
}

impl Store {
    /// Atomically creates a complete first tenant through an administrative connection.
    ///
    /// The raw SDK key is returned once and is never persisted. Receipt replays return
    /// identifiers only.
    ///
    /// # Errors
    ///
    /// Returns validation, credential, serialization, or database errors.
    pub async fn bootstrap(
        &self,
        mut request: BootstrapRequest,
    ) -> Result<BootstrapResult, ControlPlaneError> {
        let policy_version = request.normalize_and_validate()?;
        for attempt in 0_u64..3 {
            match self.bootstrap_once(&request, &policy_version).await {
                Ok(result) => return Ok(result),
                Err(error) if retryable(&error) => {
                    tokio::time::sleep(Duration::from_millis((attempt + 1) * 20)).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(ControlPlaneError::Conflict)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "bootstrap is one serializable, idempotent transaction creating the complete tenant contract"
    )]
    async fn bootstrap_once(
        &self,
        request: &BootstrapRequest,
        policy_version: &str,
    ) -> Result<BootstrapResult, ControlPlaneError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1129466196))")
            .bind(&request.idempotency_key)
            .execute(&mut *transaction)
            .await?;
        if let Some(result) = bootstrap_receipt(&mut transaction, &request.idempotency_key).await? {
            transaction.commit().await?;
            return Ok(result);
        }
        let credential = self.issuer.issue(CredentialKind::SdkKey)?;
        let schema_digest =
            Sha256::digest(serde_json::to_vec(&request.schema.definition).map_err(|_| {
                ControlPlaneError::InvalidInput("schema definition is invalid".to_owned())
            })?);
        let privacy_digest =
            Sha256::digest(serde_json::to_vec(&request.privacy.document).map_err(|_| {
                ControlPlaneError::InvalidInput("privacy document is invalid".to_owned())
            })?);
        let organization_id = sqlx::query_scalar::<_, String>(
            "INSERT INTO control.organizations (slug, name) VALUES ($1, $2) RETURNING id::text",
        )
        .bind(&request.organization.slug)
        .bind(&request.organization.name)
        .fetch_one(&mut *transaction)
        .await?;
        let owner_user_id = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.users (email, display_name) VALUES ($1, $2)
            ON CONFLICT (lower(email)) DO UPDATE SET display_name = EXCLUDED.display_name,
                status = 'active' RETURNING id::text
        ",
        )
        .bind(&request.owner.email)
        .bind(&request.owner.display_name)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.user_identities (user_id, issuer, subject)
            VALUES ($1::uuid, 'chill.email', $2)
            ON CONFLICT (issuer, subject) DO UPDATE SET user_id = EXCLUDED.user_id
        ",
        )
        .bind(&owner_user_id)
        .bind(&request.owner.email)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO control.organization_memberships (organization_id, user_id, role) \
             VALUES ($1::uuid, $2::uuid, 'owner')",
        )
        .bind(&organization_id)
        .bind(&owner_user_id)
        .execute(&mut *transaction)
        .await?;
        let project_id = sqlx::query_scalar::<_, String>(
            "INSERT INTO control.projects (organization_id, slug, name) \
             VALUES ($1::uuid, $2, $3) RETURNING id::text",
        )
        .bind(&organization_id)
        .bind(&request.project.slug)
        .bind(&request.project.name)
        .fetch_one(&mut *transaction)
        .await?;
        let environment_id = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.environments (
                organization_id, project_id, slug, name, kind, retention_days
            ) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6) RETURNING id::text
        ",
        )
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&request.environment.slug)
        .bind(&request.environment.name)
        .bind(&request.environment.kind)
        .bind(request.environment.retention_days)
        .fetch_one(&mut *transaction)
        .await?;
        let data_source_id = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.data_sources (
                organization_id, project_id, environment_id, name, kind
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5) RETURNING id::text
        ",
        )
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&environment_id)
        .bind(&request.data_source.name)
        .bind(&request.data_source.kind)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.behavior_schemas (
                organization_id, project_id, version, schema_url, digest,
                definition, compatibility
            ) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7)
        ",
        )
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&request.schema.version)
        .bind(&request.schema.url)
        .bind(schema_digest.as_slice())
        .bind(Json(request.schema.definition.clone()))
        .bind(&request.schema.compatibility)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.sampling_policies (
                organization_id, project_id, environment_id, version,
                behavior_numerator, behavior_denominator, replay_numerator,
                replay_denominator, salt_version, status, effective_at, created_by
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, 1, $4, $5, $6, $7, $8,
                'active', transaction_timestamp(), $9::uuid)
        ",
        )
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&environment_id)
        .bind(request.sampling.behavior_numerator)
        .bind(request.sampling.behavior_denominator)
        .bind(request.sampling.replay_numerator)
        .bind(request.sampling.replay_denominator)
        .bind(&request.sampling.salt_version)
        .bind(&owner_user_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.privacy_policies (
                organization_id, project_id, environment_id, version, document,
                digest, status, effective_at, created_by
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, 1, $4, $5, 'active',
                transaction_timestamp(), $6::uuid)
        ",
        )
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&environment_id)
        .bind(Json(request.privacy.document.clone()))
        .bind(privacy_digest.as_slice())
        .bind(&owner_user_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.collection_policies (
                organization_id, project_id, environment_id, revision,
                policy_version, enabled, disabled_capture_classes, created_by, idempotency_key
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, 1, $4, true, '{}', $5::uuid, $6)
        ",
        )
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&environment_id)
        .bind(policy_version)
        .bind(&owner_user_id)
        .bind(&request.idempotency_key)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.quotas (
                organization_id, project_id, environment_id, requests_per_minute,
                records_per_minute, replay_bytes_per_day, query_concurrency, query_scan_bytes
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7, $8)
        ",
        )
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&environment_id)
        .bind(request.quota.requests_per_minute)
        .bind(request.quota.records_per_minute)
        .bind(request.quota.replay_bytes_per_day)
        .bind(request.quota.query_concurrency)
        .bind(request.quota.query_scan_bytes)
        .execute(&mut *transaction)
        .await?;
        let sdk_key_id = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.sdk_keys (
                organization_id, project_id, environment_id, data_source_id,
                name, prefix, secret_digest, scopes
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7, $8)
            RETURNING id::text
        ",
        )
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&environment_id)
        .bind(&data_source_id)
        .bind(&request.sdk_key.name)
        .bind(&credential.prefix)
        .bind(credential.digest.as_slice())
        .bind(&request.sdk_key.scopes)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.bootstrap_receipts (
                idempotency_key, organization_id, project_id, environment_id,
                data_source_id, sdk_key_id, owner_user_id
            ) VALUES ($1, $2::uuid, $3::uuid, $4::uuid, $5::uuid, $6::uuid, $7::uuid)
        ",
        )
        .bind(&request.idempotency_key)
        .bind(&organization_id)
        .bind(&project_id)
        .bind(&environment_id)
        .bind(&data_source_id)
        .bind(&sdk_key_id)
        .bind(&owner_user_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                request_id, reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'control.bootstrap.created', 'organization',
                $1::uuid, $3, 'initial_setup', jsonb_build_object(
                    'project_id', $4::text, 'environment_id', $5::text))
        ",
        )
        .bind(&organization_id)
        .bind(&owner_user_id)
        .bind(&request.idempotency_key)
        .bind(&project_id)
        .bind(&environment_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(BootstrapResult {
            created: true,
            organization_id,
            project_id,
            environment_id,
            data_source_id,
            sdk_key_id,
            owner_user_id,
            sdk_key: Some(credential.raw),
        })
    }
}

async fn bootstrap_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
) -> Result<Option<BootstrapResult>, ControlPlaneError> {
    let row = sqlx::query_as::<_, (String, String, String, String, String, String)>(
        r"
        SELECT organization_id::text, project_id::text, environment_id::text,
            data_source_id::text, sdk_key_id::text, owner_user_id::text
        FROM control.bootstrap_receipts WHERE idempotency_key = $1
    ",
    )
    .bind(idempotency_key)
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(row.map(|value| BootstrapResult {
        created: false,
        organization_id: value.0,
        project_id: value.1,
        environment_id: value.2,
        data_source_id: value.3,
        sdk_key_id: value.4,
        owner_user_id: value.5,
        sdk_key: None,
    }))
}

fn valid_email(email: &str) -> bool {
    if email.is_empty() || email.len() > 320 || email.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = email.split('@');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(local), Some(domain), None)
            if !local.is_empty() && !domain.is_empty() && domain.contains('.')
    )
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ControlPlaneError> {
    Err(ControlPlaneError::InvalidInput(message.into()))
}

fn retryable(error: &ControlPlaneError) -> bool {
    matches!(
        error,
        ControlPlaneError::Database(sqlx::Error::Database(database))
            if matches!(database.code().as_deref(), Some("40001" | "40P01"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bootstrap_is_valid_and_fails_closed() {
        let mut request = BootstrapRequest::local_default(
            " OWNER@example.com ",
            serde_json::json!({"type": "object"}),
        );
        assert_eq!(
            request.normalize_and_validate().ok().as_deref(),
            Some("privacy-v1")
        );
        assert_eq!(request.owner.email, "owner@example.com");
        request.organization.slug = "NOPE".to_owned();
        assert!(request.normalize_and_validate().is_err());
    }
}
