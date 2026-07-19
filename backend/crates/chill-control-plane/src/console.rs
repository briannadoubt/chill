use std::{collections::HashMap, sync::LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::types::Json;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{Capability, ControlPlaneError, Role, Store, parse_environment_privacy_policy};

static SEMANTIC_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
        .unwrap_or_else(|error| unreachable!("static semantic-version regex is invalid: {error}"))
});

static RESOURCE_SLUG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z][a-z0-9-]{1,62}[a-z0-9]$")
        .unwrap_or_else(|error| unreachable!("static resource-slug regex is invalid: {error}"))
});

/// Authenticated console bootstrap payload.
#[derive(Debug, Serialize)]
pub struct ConsoleOverview {
    /// Current user and effective authority.
    pub actor: ConsoleActor,
    /// Current tenant.
    pub organization: ConsoleOrganization,
    /// Visible projects and their configuration.
    pub projects: Vec<ConsoleProject>,
}

/// Current console user.
#[derive(Debug, Serialize)]
pub struct ConsoleActor {
    /// User ID.
    pub id: String,
    /// Normalized account email.
    pub email: String,
    /// Display name.
    pub display_name: String,
    /// Current organization role.
    pub role: Role,
    /// Capabilities derived from the current role.
    pub capabilities: Vec<Capability>,
}

/// Current organization metadata.
#[derive(Debug, Serialize)]
pub struct ConsoleOrganization {
    /// Organization ID.
    pub id: String,
    /// Stable slug.
    pub slug: String,
    /// Display name.
    pub name: String,
}

/// Project and nested configuration.
#[derive(Debug, Serialize)]
pub struct ConsoleProject {
    /// Project ID.
    pub id: String,
    /// Stable slug.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Lifecycle status.
    pub status: String,
    /// Project environments.
    pub environments: Vec<ConsoleEnvironment>,
    /// Registered behavior schemas.
    pub schemas: Vec<ConsoleSchema>,
}

/// Environment configuration displayed by the console.
#[derive(Debug, Serialize)]
pub struct ConsoleEnvironment {
    /// Environment ID.
    pub id: String,
    /// Stable slug.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Environment class.
    pub kind: String,
    /// Lifecycle status.
    pub status: String,
    /// Behavior retention duration.
    pub retention_days: i32,
    /// Active data sources.
    pub data_sources: Vec<ConsoleDataSource>,
    /// Issued SDK keys, without secret values.
    pub sdk_keys: Vec<ConsoleSDKKey>,
    /// Active sampling policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<ConsoleSamplingPolicy>,
    /// Active privacy policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy: Option<ConsolePrivacyPolicy>,
}

/// Ingestion source registered in an environment.
#[derive(Debug, Serialize)]
pub struct ConsoleDataSource {
    /// Source ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Platform class.
    pub kind: String,
    /// Lifecycle status.
    pub status: String,
}

/// Registered behavior schema.
#[derive(Debug, Serialize)]
pub struct ConsoleSchema {
    /// Schema ID.
    pub id: String,
    /// Semantic version.
    pub version: String,
    /// Canonical schema URL.
    pub url: String,
    /// Schema JSON.
    pub definition: Value,
    /// Compatibility mode.
    pub compatibility: String,
    /// Lifecycle status.
    pub status: String,
}

/// SDK key metadata. Raw credentials are never read back.
#[derive(Debug, Serialize)]
pub struct ConsoleSDKKey {
    /// Key ID.
    pub id: String,
    /// Bound data-source ID.
    pub data_source_id: String,
    /// Display name.
    pub name: String,
    /// Non-secret lookup prefix.
    pub prefix: String,
    /// Ingestion scopes.
    pub scopes: Vec<String>,
    /// Lifecycle status.
    pub status: String,
    /// Creation timestamp in RFC 3339 UTC form.
    pub created_at: String,
    /// Optional expiry timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Optional last-use timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
}

/// Active probabilistic sampling settings.
#[derive(Debug, Serialize)]
pub struct ConsoleSamplingPolicy {
    /// Monotonic policy version.
    pub version: i64,
    /// Behavior numerator.
    pub behavior_numerator: i64,
    /// Behavior denominator.
    pub behavior_denominator: i64,
    /// Replay numerator.
    pub replay_numerator: i64,
    /// Replay denominator.
    pub replay_denominator: i64,
    /// Sampling salt identifier.
    pub salt_version: String,
}

/// Active privacy document.
#[derive(Debug, Serialize)]
pub struct ConsolePrivacyPolicy {
    /// Monotonic policy version.
    pub version: i64,
    /// Validated privacy document.
    pub document: Value,
}

/// Request to create a project in the authenticated organization.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectRequest {
    /// Stable project slug.
    pub slug: String,
    /// Human-readable project name.
    pub name: String,
}

/// Request to create an environment in an existing project.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateEnvironmentRequest {
    /// Parent project ID.
    pub project_id: String,
    /// Stable environment slug.
    pub slug: String,
    /// Human-readable environment name.
    pub name: String,
    /// Environment class.
    pub kind: String,
    /// Initial behavior retention duration.
    pub retention_days: i32,
}

/// Request to register a data source under an existing environment.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDataSourceRequest {
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// Human-readable name.
    pub name: String,
    /// One supported platform kind.
    pub kind: String,
}

/// Request to issue an environment-scoped SDK credential.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSDKKeyRequest {
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// Data-source ID.
    pub data_source_id: String,
    /// Human-readable name.
    pub name: String,
    /// Permitted ingestion scopes.
    pub scopes: Vec<String>,
    /// Optional RFC 3339 expiry.
    pub expires_at: Option<String>,
}

/// A newly issued SDK credential returned exactly once.
#[derive(Debug, Serialize)]
pub struct IssuedSDKKey {
    /// New key ID.
    pub key_id: String,
    /// One-time bearer credential.
    pub credential: String,
}

/// A replacement SDK credential returned exactly once.
#[derive(Debug, Serialize)]
pub struct RotatedSDKKey {
    /// Replacement key ID.
    pub key_id: String,
    /// One-time replacement bearer credential.
    pub credential: String,
}

/// Request to activate a new sampling-policy version.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivateSamplingRequest {
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// Behavior sample numerator.
    pub behavior_numerator: i64,
    /// Behavior sample denominator.
    pub behavior_denominator: i64,
    /// Replay sample numerator.
    pub replay_numerator: i64,
    /// Replay sample denominator.
    pub replay_denominator: i64,
    /// Deterministic sampling salt identifier.
    pub salt_version: String,
}

/// Request to validate and activate a new privacy-policy version.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivatePrivacyRequest {
    /// Project ID.
    pub project_id: String,
    /// Environment ID.
    pub environment_id: String,
    /// Complete fail-closed privacy-policy document.
    pub document: Value,
}

/// Request to register a versioned behavior schema.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSchemaRequest {
    /// Project ID.
    pub project_id: String,
    /// Semantic schema version.
    pub version: String,
    /// Canonical schema URL.
    pub url: String,
    /// JSON Schema object.
    pub definition: Value,
    /// Compatibility policy.
    pub compatibility: String,
}

impl Store {
    /// Reads the complete tenant-scoped project-console bootstrap payload.
    ///
    /// # Errors
    ///
    /// Returns authentication, authorization, RLS, or database errors.
    #[allow(
        clippy::too_many_lines,
        reason = "the overview is one ordered transaction assembling a nested consistency snapshot"
    )]
    pub async fn console_overview(
        &self,
        raw_session: &str,
    ) -> Result<ConsoleOverview, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlRead)
            .await?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let account = sqlx::query_as::<_, (String, String, String, String)>(r"
            SELECT account.email, account.display_name, organization.slug, organization.name
            FROM control.users AS account
            JOIN control.organization_memberships AS membership ON membership.user_id = account.id
            JOIN control.organizations AS organization ON organization.id = membership.organization_id
            WHERE account.id = $1::uuid AND organization.id = $2::uuid
        ").bind(&session.user_id).bind(&session.organization_id).fetch_one(&mut *transaction).await?;
        let mut overview = ConsoleOverview {
            actor: ConsoleActor {
                id: session.user_id.clone(),
                email: account.0,
                display_name: account.1,
                role: session.role,
                capabilities: session.role.capabilities(),
            },
            organization: ConsoleOrganization {
                id: session.organization_id.clone(),
                slug: account.2,
                name: account.3,
            },
            projects: Vec::new(),
        };

        let project_rows = sqlx::query_as::<_, (String, String, String, String)>(
            r"
            SELECT id::text, slug, name, status FROM control.projects
            WHERE organization_id = $1::uuid AND status <> 'deleted'
            ORDER BY lower(name), id
        ",
        )
        .bind(&session.organization_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut project_index = HashMap::new();
        for row in project_rows {
            project_index.insert(row.0.clone(), overview.projects.len());
            overview.projects.push(ConsoleProject {
                id: row.0,
                slug: row.1,
                name: row.2,
                status: row.3,
                environments: Vec::new(),
                schemas: Vec::new(),
            });
        }

        let environment_rows = sqlx::query_as::<_, (String, String, String, String, String, String, i32)>(r"
            SELECT id::text, project_id::text, slug, name, kind, status, retention_days
            FROM control.environments WHERE organization_id = $1::uuid AND status <> 'deleted'
            ORDER BY CASE kind WHEN 'production' THEN 0 WHEN 'staging' THEN 1 WHEN 'development' THEN 2 ELSE 3 END, lower(name), id
        ").bind(&session.organization_id).fetch_all(&mut *transaction).await?;
        let mut environment_index = HashMap::new();
        for row in environment_rows {
            if let Some(&project) = project_index.get(&row.1) {
                let environment = overview.projects[project].environments.len();
                environment_index.insert(row.0.clone(), (project, environment));
                overview.projects[project]
                    .environments
                    .push(ConsoleEnvironment {
                        id: row.0,
                        slug: row.2,
                        name: row.3,
                        kind: row.4,
                        status: row.5,
                        retention_days: row.6,
                        data_sources: Vec::new(),
                        sdk_keys: Vec::new(),
                        sampling: None,
                        privacy: None,
                    });
            }
        }

        let source_rows = sqlx::query_as::<_, (String, String, String, String, String)>(
            r"
            SELECT id::text, environment_id::text, name, kind, status FROM control.data_sources
            WHERE organization_id = $1::uuid AND status <> 'deleted' ORDER BY lower(name), id
        ",
        )
        .bind(&session.organization_id)
        .fetch_all(&mut *transaction)
        .await?;
        for row in source_rows {
            if let Some(&(project, environment)) = environment_index.get(&row.1) {
                overview.projects[project].environments[environment]
                    .data_sources
                    .push(ConsoleDataSource {
                        id: row.0,
                        name: row.2,
                        kind: row.3,
                        status: row.4,
                    });
            }
        }

        let schema_rows = sqlx::query_as::<_, (String, String, String, String, Json<Value>, String, String)>(r"
            SELECT id::text, project_id::text, version, schema_url, definition, compatibility, status
            FROM control.behavior_schemas WHERE organization_id = $1::uuid ORDER BY created_at DESC, id
        ").bind(&session.organization_id).fetch_all(&mut *transaction).await?;
        for row in schema_rows {
            if let Some(&project) = project_index.get(&row.1) {
                overview.projects[project].schemas.push(ConsoleSchema {
                    id: row.0,
                    version: row.2,
                    url: row.3,
                    definition: row.4.0,
                    compatibility: row.5,
                    status: row.6,
                });
            }
        }

        let key_rows = sqlx::query_as::<_, (String, String, String, String, String, Vec<String>, String, String, Option<String>, Option<String>)>(r#"
            SELECT id::text, environment_id::text, data_source_id::text, name, prefix, scopes, status,
                   to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
                   CASE WHEN expires_at IS NULL THEN NULL ELSE to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') END,
                   CASE WHEN last_used_at IS NULL THEN NULL ELSE to_char(last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') END
            FROM control.sdk_keys WHERE organization_id = $1::uuid ORDER BY created_at DESC, id
        "#).bind(&session.organization_id).fetch_all(&mut *transaction).await?;
        for row in key_rows {
            if let Some(&(project, environment)) = environment_index.get(&row.1) {
                overview.projects[project].environments[environment]
                    .sdk_keys
                    .push(ConsoleSDKKey {
                        id: row.0,
                        data_source_id: row.2,
                        name: row.3,
                        prefix: row.4,
                        scopes: row.5,
                        status: row.6,
                        created_at: row.7,
                        expires_at: row.8,
                        last_used_at: row.9,
                    });
            }
        }

        let sampling_rows = sqlx::query_as::<_, (String, i64, i64, i64, i64, i64, String)>(r"
            SELECT environment_id::text, version, behavior_numerator, behavior_denominator, replay_numerator, replay_denominator, salt_version
            FROM control.sampling_policies WHERE organization_id = $1::uuid AND status = 'active'
        ").bind(&session.organization_id).fetch_all(&mut *transaction).await?;
        for row in sampling_rows {
            if let Some(&(project, environment)) = environment_index.get(&row.0) {
                overview.projects[project].environments[environment].sampling =
                    Some(ConsoleSamplingPolicy {
                        version: row.1,
                        behavior_numerator: row.2,
                        behavior_denominator: row.3,
                        replay_numerator: row.4,
                        replay_denominator: row.5,
                        salt_version: row.6,
                    });
            }
        }

        let privacy_rows = sqlx::query_as::<_, (String, i64, Json<Value>)>(
            r"
            SELECT environment_id::text, version, document FROM control.privacy_policies
            WHERE organization_id = $1::uuid AND status = 'active'
        ",
        )
        .bind(&session.organization_id)
        .fetch_all(&mut *transaction)
        .await?;
        for row in privacy_rows {
            if let Some(&(project, environment)) = environment_index.get(&row.0) {
                overview.projects[project].environments[environment].privacy =
                    Some(ConsolePrivacyPolicy {
                        version: row.1,
                        document: row.2.0,
                    });
            }
        }
        transaction.commit().await?;
        Ok(overview)
    }

    /// Updates an active environment's retention and appends an audit record.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, RLS, or database errors.
    pub async fn update_environment_retention(
        &self,
        raw_session: &str,
        environment_id: &str,
        retention_days: i32,
    ) -> Result<(), ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlWrite)
            .await?;
        Uuid::parse_str(environment_id).map_err(|_| {
            ControlPlaneError::InvalidInput("environment must be a UUID".to_owned())
        })?;
        if !(1..=3650).contains(&retention_days) {
            return Err(ControlPlaneError::InvalidInput(
                "retention days must be between 1 and 3650".to_owned(),
            ));
        }
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let updated = sqlx::query("UPDATE control.environments SET retention_days = $2, updated_at = clock_timestamp() WHERE id = $1::uuid AND status = 'active'")
            .bind(environment_id).bind(retention_days).execute(&mut *transaction).await?;
        if updated.rows_affected() != 1 {
            return Err(ControlPlaneError::Forbidden);
        }
        sqlx::query(r"
            INSERT INTO control.audit_log (organization_id, actor_user_id, action, target_type, target_id, reason_code, details)
            VALUES ($1::uuid, $2::uuid, 'environment.retention.updated', 'environment', $3::uuid, 'user_requested', jsonb_build_object('retention_days', $4::integer))
        ").bind(&session.organization_id).bind(&session.user_id).bind(environment_id).bind(retention_days).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Creates an active project in the authenticated organization.
    ///
    /// # Errors
    ///
    /// Returns validation, conflict, authorization, RLS, or database errors.
    pub async fn create_project(
        &self,
        raw_session: &str,
        mut request: CreateProjectRequest,
    ) -> Result<ConsoleProject, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlWrite)
            .await?;
        request.slug = request.slug.trim().to_owned();
        request.name = request.name.trim().to_owned();
        if !RESOURCE_SLUG.is_match(&request.slug)
            || request.name.is_empty()
            || request.name.len() > 160
        {
            return Err(ControlPlaneError::InvalidInput(
                "project is invalid".to_owned(),
            ));
        }
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let insert = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.projects (organization_id, slug, name)
            VALUES ($1::uuid, $2, $3)
            RETURNING id::text
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.slug)
        .bind(&request.name)
        .fetch_one(&mut *transaction)
        .await;
        let project_id = match insert {
            Ok(id) => id,
            Err(error) if is_unique_violation(&error) => return Err(ControlPlaneError::Conflict),
            Err(error) => return Err(error.into()),
        };
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id, reason_code
            ) VALUES ($1::uuid, $2::uuid, 'project.created', 'project', $3::uuid,
                'user_requested')
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(&project_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ConsoleProject {
            id: project_id,
            slug: request.slug,
            name: request.name,
            status: "active".to_owned(),
            environments: Vec::new(),
            schemas: Vec::new(),
        })
    }

    /// Creates an active environment in an authenticated tenant project.
    ///
    /// # Errors
    ///
    /// Returns validation, conflict, authorization, RLS, or database errors.
    pub async fn create_environment(
        &self,
        raw_session: &str,
        mut request: CreateEnvironmentRequest,
    ) -> Result<ConsoleEnvironment, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlWrite)
            .await?;
        validate_uuid("project", &request.project_id)?;
        request.slug = request.slug.trim().to_owned();
        request.name = request.name.trim().to_owned();
        if !RESOURCE_SLUG.is_match(&request.slug)
            || request.name.is_empty()
            || request.name.len() > 160
            || !["production", "staging", "development", "test"].contains(&request.kind.as_str())
            || !(1..=3650).contains(&request.retention_days)
        {
            return Err(ControlPlaneError::InvalidInput(
                "environment is invalid".to_owned(),
            ));
        }
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let insert = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.environments (
                organization_id, project_id, slug, name, kind, retention_days
            )
            SELECT $1::uuid, id, $3, $4, $5, $6 FROM control.projects
            WHERE organization_id = $1::uuid AND id = $2::uuid AND status = 'active'
            RETURNING id::text
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.slug)
        .bind(&request.name)
        .bind(&request.kind)
        .bind(request.retention_days)
        .fetch_optional(&mut *transaction)
        .await;
        let environment_id = match insert {
            Ok(Some(id)) => id,
            Ok(None) => return Err(ControlPlaneError::Forbidden),
            Err(error) if is_unique_violation(&error) => return Err(ControlPlaneError::Conflict),
            Err(error) => return Err(error.into()),
        };
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'environment.created', 'environment',
                $3::uuid, 'user_requested', jsonb_build_object('kind', $4::text))
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(&environment_id)
        .bind(&request.kind)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ConsoleEnvironment {
            id: environment_id,
            slug: request.slug,
            name: request.name,
            kind: request.kind,
            status: "active".to_owned(),
            retention_days: request.retention_days,
            data_sources: Vec::new(),
            sdk_keys: Vec::new(),
            sampling: None,
            privacy: None,
        })
    }

    /// Registers one supported client or OTLP data source.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, RLS, or database errors.
    pub async fn create_data_source(
        &self,
        raw_session: &str,
        mut request: CreateDataSourceRequest,
    ) -> Result<ConsoleDataSource, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlWrite)
            .await?;
        Uuid::parse_str(&request.project_id)
            .map_err(|_| ControlPlaneError::InvalidInput("project must be a UUID".to_owned()))?;
        Uuid::parse_str(&request.environment_id).map_err(|_| {
            ControlPlaneError::InvalidInput("environment must be a UUID".to_owned())
        })?;
        request.name = request.name.trim().to_owned();
        if request.name.is_empty()
            || request.name.len() > 160
            || !["apple", "android", "web", "server", "otlp"].contains(&request.kind.as_str())
        {
            return Err(ControlPlaneError::InvalidInput(
                "data source is invalid".to_owned(),
            ));
        }
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let id = sqlx::query_scalar::<_, String>(r"
            INSERT INTO control.data_sources (organization_id, project_id, environment_id, name, kind)
            SELECT $1::uuid, project.id, environment.id, $4, $5 FROM control.projects AS project
            JOIN control.environments AS environment ON environment.project_id = project.id AND environment.organization_id = project.organization_id
            WHERE project.organization_id = $1::uuid AND project.id = $2::uuid AND environment.id = $3::uuid
              AND project.status = 'active' AND environment.status = 'active' RETURNING id::text
        ").bind(&session.organization_id).bind(&request.project_id).bind(&request.environment_id).bind(&request.name).bind(&request.kind)
          .fetch_optional(&mut *transaction).await?.ok_or(ControlPlaneError::Forbidden)?;
        sqlx::query(r"
            INSERT INTO control.audit_log (organization_id, actor_user_id, action, target_type, target_id, reason_code, details)
            VALUES ($1::uuid, $2::uuid, 'data_source.created', 'data_source', $3::uuid, 'user_requested', jsonb_build_object('kind', $4::text))
        ").bind(&session.organization_id).bind(&session.user_id).bind(&id).bind(&request.kind).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(ConsoleDataSource {
            id,
            name: request.name,
            kind: request.kind,
            status: "active".to_owned(),
        })
    }

    /// Issues an SDK key bound to one active tenant data source.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, credential, RLS, or database errors.
    pub async fn create_sdk_key(
        &self,
        raw_session: &str,
        mut request: CreateSDKKeyRequest,
    ) -> Result<IssuedSDKKey, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::CredentialsManage)
            .await?;
        validate_uuid("project", &request.project_id)?;
        validate_uuid("environment", &request.environment_id)?;
        validate_uuid("data source", &request.data_source_id)?;
        request.name = request.name.trim().to_owned();
        if request.name.is_empty() || request.name.len() > 160 {
            return Err(ControlPlaneError::InvalidInput(
                "SDK key is invalid".to_owned(),
            ));
        }
        if request.scopes.is_empty()
            || request.scopes.len() > 2
            || request
                .scopes
                .iter()
                .any(|scope| scope != "ingest:otlp" && scope != "ingest:replay")
        {
            return Err(ControlPlaneError::InvalidInput(
                "SDK key scope is invalid".to_owned(),
            ));
        }
        let expires_at = request
            .expires_at
            .as_deref()
            .map(|value| {
                OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
                    ControlPlaneError::InvalidInput("SDK key expiry is invalid".to_owned())
                })
            })
            .transpose()?;
        if expires_at.is_some_and(|value| value <= OffsetDateTime::now_utc()) {
            return Err(ControlPlaneError::InvalidInput(
                "SDK key expiry is invalid".to_owned(),
            ));
        }
        let credential = self.issuer.issue(crate::CredentialKind::SdkKey)?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let key_id = sqlx::query_scalar::<_, String>(r"
            INSERT INTO control.sdk_keys (
                organization_id, project_id, environment_id, data_source_id,
                name, prefix, secret_digest, scopes, expires_at
            )
            SELECT $1::uuid, project.id, environment.id, source.id, $5, $6, $7, $8, $9
            FROM control.projects AS project
            JOIN control.environments AS environment
              ON environment.project_id = project.id AND environment.organization_id = project.organization_id
            JOIN control.data_sources AS source
              ON source.environment_id = environment.id AND source.project_id = project.id AND source.organization_id = project.organization_id
            WHERE project.organization_id = $1::uuid AND project.id = $2::uuid
              AND environment.id = $3::uuid AND source.id = $4::uuid
              AND project.status = 'active' AND environment.status = 'active' AND source.status = 'active'
            RETURNING id::text
        ")
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .bind(&request.data_source_id)
        .bind(&request.name)
        .bind(&credential.prefix)
        .bind(credential.digest.as_slice())
        .bind(&request.scopes)
        .bind(expires_at)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ControlPlaneError::Forbidden)?;
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'credential.sdk.created', 'sdk_key', $3::uuid,
                'user_requested', jsonb_build_object('scopes', $4::text[]))
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(&key_id)
        .bind(&request.scopes)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(IssuedSDKKey {
            key_id,
            credential: credential.raw,
        })
    }

    /// Atomically replaces an active SDK key and revokes its predecessor.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, credential, RLS, or database errors.
    pub async fn rotate_sdk_key(
        &self,
        raw_session: &str,
        key_id: &str,
    ) -> Result<RotatedSDKKey, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::CredentialsManage)
            .await?;
        validate_uuid("SDK key", key_id)?;
        let credential = self.issuer.issue(crate::CredentialKind::SdkKey)?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let source = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                Vec<String>,
                Option<OffsetDateTime>,
            ),
        >(
            r"
            SELECT project_id::text, environment_id::text, data_source_id::text,
                   name, scopes, expires_at
            FROM control.sdk_keys
            WHERE id = $1::uuid AND status = 'active'
              AND (expires_at IS NULL OR expires_at > statement_timestamp())
            FOR UPDATE
        ",
        )
        .bind(key_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ControlPlaneError::Forbidden)?;
        let replacement_id = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.sdk_keys (
                organization_id, project_id, environment_id, data_source_id,
                name, prefix, secret_digest, scopes, expires_at, rotated_from_id
            ) VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7, $8, $9, $10::uuid)
            RETURNING id::text
        ",
        )
        .bind(&session.organization_id)
        .bind(&source.0)
        .bind(&source.1)
        .bind(&source.2)
        .bind(&source.3)
        .bind(&credential.prefix)
        .bind(credential.digest.as_slice())
        .bind(&source.4)
        .bind(source.5)
        .bind(key_id)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            UPDATE control.sdk_keys SET status = 'revoked', revoked_at = clock_timestamp(),
                replaced_by_id = $2::uuid WHERE id = $1::uuid AND status = 'active'
        ",
        )
        .bind(key_id)
        .bind(&replacement_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'credential.sdk.rotated', 'sdk_key', $3::uuid,
                'user_requested', jsonb_build_object('replacement_id', $4::text))
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(key_id)
        .bind(&replacement_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(RotatedSDKKey {
            key_id: replacement_id,
            credential: credential.raw,
        })
    }

    /// Revokes an active SDK key and appends an audit record.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, RLS, or database errors.
    pub async fn revoke_sdk_key(
        &self,
        raw_session: &str,
        key_id: &str,
    ) -> Result<(), ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::CredentialsManage)
            .await?;
        validate_uuid("SDK key", key_id)?;
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let updated = sqlx::query(
            r"
            UPDATE control.sdk_keys SET status = 'revoked', revoked_at = clock_timestamp()
            WHERE id = $1::uuid AND status = 'active'
        ",
        )
        .bind(key_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ControlPlaneError::Forbidden);
        }
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id, reason_code
            ) VALUES ($1::uuid, $2::uuid, 'credential.sdk.revoked', 'sdk_key', $3::uuid,
                'user_requested')
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(key_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Retires the current sampling policy and activates its successor atomically.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, RLS, or database errors.
    pub async fn activate_sampling_policy(
        &self,
        raw_session: &str,
        request: ActivateSamplingRequest,
    ) -> Result<(), ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlWrite)
            .await?;
        validate_uuid("project", &request.project_id)?;
        validate_uuid("environment", &request.environment_id)?;
        if request.behavior_denominator <= 0
            || request.behavior_numerator < 0
            || request.behavior_numerator > request.behavior_denominator
        {
            return Err(ControlPlaneError::InvalidInput(
                "behavior sampling ratio is invalid".to_owned(),
            ));
        }
        if request.replay_denominator <= 0
            || request.replay_numerator < 0
            || request.replay_numerator > request.replay_denominator
        {
            return Err(ControlPlaneError::InvalidInput(
                "replay sampling ratio is invalid".to_owned(),
            ));
        }
        if request.salt_version.is_empty() || request.salt_version.len() > 64 {
            return Err(ControlPlaneError::InvalidInput(
                "sampling salt version is invalid".to_owned(),
            ));
        }
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let version = sqlx::query_scalar::<_, i64>(
            r"
            SELECT COALESCE(max(version), 0)::bigint + 1 FROM control.sampling_policies
            WHERE project_id = $1::uuid AND environment_id = $2::uuid
        ",
        )
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            UPDATE control.sampling_policies SET status = 'retired'
            WHERE project_id = $1::uuid AND environment_id = $2::uuid AND status = 'active'
        ",
        )
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .execute(&mut *transaction)
        .await?;
        let inserted = sqlx::query(r"
            INSERT INTO control.sampling_policies (
                organization_id, project_id, environment_id, version,
                behavior_numerator, behavior_denominator, replay_numerator,
                replay_denominator, salt_version, status, effective_at, created_by
            )
            SELECT $1::uuid, project.id, environment.id, $4, $5, $6, $7, $8, $9,
                'active', transaction_timestamp(), $10::uuid
            FROM control.projects AS project
            JOIN control.environments AS environment
              ON environment.project_id = project.id AND environment.organization_id = project.organization_id
            WHERE project.organization_id = $1::uuid AND project.id = $2::uuid
              AND environment.id = $3::uuid AND project.status = 'active'
              AND environment.status = 'active'
        ")
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .bind(version)
        .bind(request.behavior_numerator)
        .bind(request.behavior_denominator)
        .bind(request.replay_numerator)
        .bind(request.replay_denominator)
        .bind(&request.salt_version)
        .bind(&session.user_id)
        .execute(&mut *transaction)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(ControlPlaneError::Forbidden);
        }
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'sampling_policy.activated', 'environment',
                $3::uuid, 'user_requested', jsonb_build_object('version', $4::bigint))
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(&request.environment_id)
        .bind(version)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Validates and activates a privacy policy and matching collection policy atomically.
    ///
    /// # Errors
    ///
    /// Returns validation, authorization, RLS, or database errors.
    #[allow(
        clippy::too_many_lines,
        reason = "privacy and collection policy activation is one auditable atomic transaction"
    )]
    pub async fn activate_privacy_policy(
        &self,
        raw_session: &str,
        request: ActivatePrivacyRequest,
    ) -> Result<(), ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlWrite)
            .await?;
        validate_uuid("project", &request.project_id)?;
        validate_uuid("environment", &request.environment_id)?;
        let policy = parse_environment_privacy_policy(request.document.clone())?;
        let canonical = serde_json::to_vec(&request.document).map_err(|_| {
            ControlPlaneError::InvalidInput("privacy document is invalid".to_owned())
        })?;
        let digest = Sha256::digest(&canonical);
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let version = sqlx::query_scalar::<_, i64>(
            r"
            SELECT COALESCE(max(version), 0)::bigint + 1 FROM control.privacy_policies
            WHERE project_id = $1::uuid AND environment_id = $2::uuid
        ",
        )
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            UPDATE control.privacy_policies SET status = 'retired'
            WHERE project_id = $1::uuid AND environment_id = $2::uuid AND status = 'active'
        ",
        )
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .execute(&mut *transaction)
        .await?;
        let inserted = sqlx::query(
            r"
            INSERT INTO control.privacy_policies (
                organization_id, project_id, environment_id, version, document,
                digest, status, effective_at, created_by
            )
            SELECT $1::uuid, project.id, environment.id, $4, $5, $6,
                'active', transaction_timestamp(), $7::uuid
            FROM control.projects AS project
            JOIN control.environments AS environment
              ON environment.project_id = project.id
             AND environment.organization_id = project.organization_id
            WHERE project.organization_id = $1::uuid AND project.id = $2::uuid
              AND environment.id = $3::uuid AND project.status = 'active'
              AND environment.status = 'active'
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .bind(version)
        .bind(Json(request.document))
        .bind(digest.as_slice())
        .bind(&session.user_id)
        .execute(&mut *transaction)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(ControlPlaneError::Forbidden);
        }
        sqlx::query(
            "UPDATE control.collection_policies SET status = 'retired' \
             WHERE environment_id = $1::uuid AND status = 'active'",
        )
        .bind(&request.environment_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.collection_policies (
                organization_id, project_id, environment_id, revision,
                policy_version, enabled, disabled_capture_classes, created_by, idempotency_key
            )
            SELECT $1::uuid, $2::uuid, $3::uuid, COALESCE(max(revision), 0) + 1,
                $4, true, '{}', $5::uuid, $6
            FROM control.collection_policies WHERE environment_id = $3::uuid
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.environment_id)
        .bind(&policy.policy_version)
        .bind(&session.user_id)
        .bind(format!(
            "console-privacy-{}-{version}",
            request.environment_id
        ))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'privacy_policy.activated', 'environment',
                $3::uuid, 'user_requested', jsonb_build_object(
                    'version', $4::bigint, 'policy_version', $5::text))
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(&request.environment_id)
        .bind(version)
        .bind(&policy.policy_version)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Registers an immutable behavior-schema version for an active project.
    ///
    /// # Errors
    ///
    /// Returns validation, conflict, authorization, RLS, or database errors.
    pub async fn create_behavior_schema(
        &self,
        raw_session: &str,
        request: CreateSchemaRequest,
    ) -> Result<ConsoleSchema, ControlPlaneError> {
        let session = self
            .require_user_capability(raw_session, Capability::ControlWrite)
            .await?;
        validate_uuid("project", &request.project_id)?;
        if !SEMANTIC_VERSION.is_match(&request.version) {
            return Err(ControlPlaneError::InvalidInput(
                "schema version must be semantic x.y.z".to_owned(),
            ));
        }
        if request.url.is_empty() || request.url.len() > 2048 {
            return Err(ControlPlaneError::InvalidInput(
                "schema URL is invalid".to_owned(),
            ));
        }
        if !["exact", "backward", "forward", "full"].contains(&request.compatibility.as_str()) {
            return Err(ControlPlaneError::InvalidInput(
                "schema compatibility is invalid".to_owned(),
            ));
        }
        if !request.definition.is_object() {
            return Err(ControlPlaneError::InvalidInput(
                "schema definition must be a JSON object".to_owned(),
            ));
        }
        let canonical = serde_json::to_vec(&request.definition).map_err(|_| {
            ControlPlaneError::InvalidInput("schema definition is invalid".to_owned())
        })?;
        let digest = Sha256::digest(&canonical);
        let mut transaction = self.begin_tenant(&session.organization_id).await?;
        let insert = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO control.behavior_schemas (
                organization_id, project_id, version, schema_url, digest,
                definition, compatibility
            )
            SELECT $1::uuid, id, $3, $4, $5, $6, $7 FROM control.projects
            WHERE organization_id = $1::uuid AND id = $2::uuid AND status = 'active'
            RETURNING id::text
        ",
        )
        .bind(&session.organization_id)
        .bind(&request.project_id)
        .bind(&request.version)
        .bind(&request.url)
        .bind(digest.as_slice())
        .bind(Json(request.definition.clone()))
        .bind(&request.compatibility)
        .fetch_optional(&mut *transaction)
        .await;
        let schema_id = match insert {
            Ok(Some(id)) => id,
            Ok(None) => return Err(ControlPlaneError::Forbidden),
            Err(error) if is_unique_violation(&error) => return Err(ControlPlaneError::Conflict),
            Err(error) => return Err(error.into()),
        };
        sqlx::query(
            r"
            INSERT INTO control.audit_log (
                organization_id, actor_user_id, action, target_type, target_id,
                reason_code, details
            ) VALUES ($1::uuid, $2::uuid, 'behavior_schema.created', 'behavior_schema',
                $3::uuid, 'user_requested', jsonb_build_object('version', $4::text))
        ",
        )
        .bind(&session.organization_id)
        .bind(&session.user_id)
        .bind(&schema_id)
        .bind(&request.version)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ConsoleSchema {
            id: schema_id,
            version: request.version,
            url: request.url,
            definition: request.definition,
            compatibility: request.compatibility,
            status: "active".to_owned(),
        })
    }
}

fn validate_uuid(label: &str, value: &str) -> Result<(), ControlPlaneError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ControlPlaneError::InvalidInput(format!("{label} must be a UUID")))
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.code().as_deref() == Some("23505"))
}
