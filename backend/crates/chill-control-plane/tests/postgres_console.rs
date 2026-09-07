//! `PostgreSQL` integration coverage for Rust console operations and tenant isolation.

use std::{env, time::Duration};

use anyhow::{Context as _, Result};
use axum::{
    body::Body,
    http::{
        Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
};
use chill_control_plane::{
    ActivatePrivacyRequest, ActivateSamplingRequest, AnalyticsScope, BootstrapRequest, Capability,
    ControlPlaneError, CreateAlertRequest, CreateDashboardRequest, CreateDataSourceRequest,
    CreateEnvironmentRequest, CreateProjectRequest, CreateSDKKeyRequest, CreateSavedQueryRequest,
    CreateSchemaRequest, CredentialIssuer, CredentialKind, ServiceCredentialRequest,
    SetCollectionPolicyRequest, SitesAuthentication, Store, UpdateDashboardRequest,
    UpdateSavedQueryRequest, VerifiedIdentity, collection_router, console_router,
    parse_credential_prefix, sites_auth_router,
};
use http_body_util::BodyExt as _;
use sqlx::{Executor as _, PgPool, postgres::PgPoolOptions};
use tower::ServiceExt as _;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires CHILL_TEST_DATABASE_URL and PostgreSQL"]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end scenario proves ordered credential and tenant state transitions"
)]
async fn console_routes_preserve_tenant_isolation() -> Result<()> {
    let database_url =
        env::var("CHILL_TEST_DATABASE_URL").context("CHILL_TEST_DATABASE_URL is required")?;
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .context("connect admin pool")?;
    chill_migrations::migrate(&admin)
        .await
        .context("apply migrations")?;

    let issuer = CredentialIssuer::new(&[0x42; 32]).context("create issuer")?;
    let session = issuer
        .issue(CredentialKind::UserSession)
        .context("issue session")?;
    let fixture = seed_fixture(&admin, &session.prefix, &session.digest).await?;

    let runtime = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                connection.execute("SET ROLE chill_app").await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .context("connect runtime pool")?;
    let admin_store = Store::new(admin.clone(), issuer.clone());
    let store = Store::new(runtime, issuer);

    let bootstrap_suffix = Uuid::new_v4().simple().to_string();
    let mut bootstrap_request = BootstrapRequest::local_default(
        &format!("bootstrap-{bootstrap_suffix}@example.invalid"),
        serde_json::json!({"type": "object"}),
    );
    bootstrap_request.idempotency_key = format!("rust-bootstrap-{bootstrap_suffix}");
    bootstrap_request.organization.slug = format!("rust-boot-{}", &bootstrap_suffix[..12]);
    bootstrap_request.project.slug = format!("project-{}", &bootstrap_suffix[12..24]);
    let first_bootstrap = admin_store
        .bootstrap(bootstrap_request.clone())
        .await
        .context("bootstrap Rust tenant")?;
    assert!(first_bootstrap.created);
    let bootstrap_key = first_bootstrap
        .sdk_key
        .as_deref()
        .context("initial bootstrap key missing")?;
    let authenticated_bootstrap_key = store
        .authenticate_sdk_key(bootstrap_key)
        .await
        .context("authenticate bootstrap SDK key")?;
    assert_eq!(
        authenticated_bootstrap_key.organization_id,
        first_bootstrap.organization_id
    );
    let replayed_bootstrap = admin_store
        .bootstrap(bootstrap_request)
        .await
        .context("replay Rust bootstrap")?;
    assert!(!replayed_bootstrap.created);
    assert!(replayed_bootstrap.sdk_key.is_none());
    assert_eq!(
        replayed_bootstrap.organization_id,
        first_bootstrap.organization_id
    );

    let response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .uri("/v1/console/overview")
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        response
            .headers()
            .get("x-frame-options")
            .and_then(|value| value.to_str().ok()),
        Some("DENY")
    );
    let response_body = response.into_body().collect().await?.to_bytes();
    let response_json: serde_json::Value = serde_json::from_slice(&response_body)?;
    assert_eq!(response_json["organization"]["id"], fixture.organization);

    let sites_secret = "01234567890123456789012345678901";
    let exchange_response = sites_auth_router(
        store.clone(),
        SitesAuthentication::new(sites_secret).context("create Sites authentication")?,
    )
    .oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/auth/sites-session")
            .header(CONTENT_TYPE, "application/json")
            .header("x-chill-sites-auth", sites_secret)
            .body(Body::from(
                serde_json::json!({ "email": &fixture.owner_email }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(exchange_response.status(), StatusCode::CREATED);
    assert_eq!(
        exchange_response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let exchange_body = exchange_response.into_body().collect().await?.to_bytes();
    let exchange_json: serde_json::Value = serde_json::from_slice(&exchange_body)?;
    assert!(exchange_json["expiresAt"].is_string());
    let exchanged_credential = exchange_json["credential"]
        .as_str()
        .context("Sites exchange credential missing")?;
    let exchanged_session = store
        .authenticate_user_session(exchanged_credential)
        .await
        .context("authenticate Sites exchange credential")?;
    assert_eq!(exchanged_session.organization_id, fixture.organization);

    let rejected_exchange = sites_auth_router(
        store.clone(),
        SitesAuthentication::new(sites_secret).context("create Sites authentication")?,
    )
    .oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/auth/sites-session")
            .header(CONTENT_TYPE, "application/json")
            .header("x-chill-sites-auth", "11234567890123456789012345678901")
            .body(Body::from(
                serde_json::json!({ "email": &fixture.owner_email }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(rejected_exchange.status(), StatusCode::UNAUTHORIZED);

    let overview = store
        .console_overview(&session.raw)
        .await
        .context("read overview")?;
    assert_eq!(overview.organization.id, fixture.organization);
    assert_eq!(overview.projects.len(), 1);
    assert_eq!(overview.projects[0].environments.len(), 1);

    let saved_query = store
        .create_saved_query(
            &session.raw,
            CreateSavedQueryRequest {
                project_id: fixture.project.clone(),
                environment_id: fixture.environment.clone(),
                name: "Signup conversion".to_owned(),
                description: "Internal release signal".to_owned(),
                plan: serde_json::json!({
                    "version": 1,
                    "kind": "aggregate",
                    "range": {"start_unix_nano": 1, "end_unix_nano": 2},
                    "aggregate": {
                        "metric": "count",
                        "dimension": "none",
                        "interval": "day",
                        "filter": {"behavior_kind": "", "operation": "", "name": "user.signed_up", "annotations": {}},
                        "limit": 30
                    }
                }),
                visualization: "line".to_owned(),
            },
        )
        .await
        .context("create saved analytics query")?;
    let dashboard = store
        .create_dashboard(
            &session.raw,
            CreateDashboardRequest {
                project_id: fixture.project.clone(),
                environment_id: fixture.environment.clone(),
                name: "Product health".to_owned(),
                description: "Shared internal dashboard".to_owned(),
                sharing: "organization".to_owned(),
                layout: serde_json::json!([{"saved_query_id": saved_query.id.clone(), "width": 2}]),
            },
        )
        .await
        .context("create analytics dashboard")?;
    let alert = store
        .create_alert(
            &session.raw,
            CreateAlertRequest {
                project_id: fixture.project.clone(),
                environment_id: fixture.environment.clone(),
                saved_query_id: saved_query.id.clone(),
                name: "Signup stalled".to_owned(),
                operator: "lte".to_owned(),
                threshold: 0.0,
                schedule_minutes: 15,
            },
        )
        .await
        .context("create scheduled alert")?;
    let updated_query = store
        .update_saved_query(
            &session.raw,
            &saved_query.id,
            UpdateSavedQueryRequest {
                name: "Signup conversion — rolling".to_owned(),
                description: "Seven-day release signal".to_owned(),
                plan: serde_json::json!({
                    "version": 1,
                    "kind": "aggregate",
                    "range": {
                        "start_unix_nano": 1,
                        "end_unix_nano": 2,
                        "relative": {"amount": 7, "unit": "day"}
                    },
                    "aggregate": {
                        "metric": "count",
                        "dimension": "none",
                        "interval": "day",
                        "filter": {"behavior_kind": "", "operation": "", "name": "user.signed_up", "annotations": {}},
                        "limit": 30
                    }
                }),
                visualization: "bar".to_owned(),
            },
        )
        .await
        .context("update saved analytics query")?;
    assert_eq!(updated_query.name, "Signup conversion — rolling");
    assert_eq!(updated_query.visualization, "bar");
    assert_eq!(updated_query.plan["range"]["relative"]["amount"], 7);
    let updated_dashboard = store
        .update_dashboard(
            &session.raw,
            &dashboard.id,
            UpdateDashboardRequest {
                name: "Release health".to_owned(),
                description: "Shared rolling release dashboard".to_owned(),
                sharing: "private".to_owned(),
                layout: serde_json::json!([{
                    "saved_query_id": saved_query.id.clone(),
                    "width": 1
                }]),
            },
        )
        .await
        .context("update analytics dashboard")?;
    assert_eq!(updated_dashboard.name, "Release health");
    assert_eq!(updated_dashboard.sharing, "private");
    assert_eq!(updated_dashboard.layout[0]["width"], 1);
    let analytics = store
        .analytics_workspace(
            &session.raw,
            AnalyticsScope {
                project_id: fixture.project.clone(),
                environment_id: fixture.environment.clone(),
            },
        )
        .await
        .context("read analytics workspace")?;
    assert_eq!(analytics.saved_queries.len(), 1);
    assert_eq!(analytics.dashboards[0].id, dashboard.id);
    assert_eq!(
        analytics.saved_queries[0].name,
        "Signup conversion — rolling"
    );
    assert_eq!(analytics.dashboards[0].name, "Release health");
    assert_eq!(analytics.alerts[0].id, alert.id);
    let debugger = store
        .debugger_snapshot(
            &session.raw,
            AnalyticsScope {
                project_id: fixture.project.clone(),
                environment_id: fixture.environment.clone(),
            },
        )
        .await
        .context("read live debugger")?;
    assert!(debugger.requests.is_empty());
    assert!(debugger.sources.is_empty());
    assert!(matches!(
        store
            .create_alert(
                &session.raw,
                CreateAlertRequest {
                    project_id: fixture.project.clone(),
                    environment_id: fixture.other_environment.clone(),
                    saved_query_id: saved_query.id,
                    name: "Cross tenant".to_owned(),
                    operator: "gt".to_owned(),
                    threshold: 1.0,
                    schedule_minutes: 15,
                },
            )
            .await,
        Err(ControlPlaneError::Forbidden)
    ));

    let project_body = serde_json::to_vec(&CreateProjectRequest {
        slug: format!("console-{}", &bootstrap_suffix[..12]),
        name: "Console-created project".to_owned(),
    })?;
    let project_response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/console/projects")
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .header("content-type", "application/json")
                .body(Body::from(project_body))?,
        )
        .await?;
    assert_eq!(project_response.status(), StatusCode::CREATED);
    let project_json: serde_json::Value =
        serde_json::from_slice(&project_response.into_body().collect().await?.to_bytes())?;
    let created_project_id = project_json["id"]
        .as_str()
        .context("created project ID missing")?;

    let environment_body = serde_json::to_vec(&CreateEnvironmentRequest {
        project_id: created_project_id.to_owned(),
        slug: "production".to_owned(),
        name: "Production".to_owned(),
        kind: "production".to_owned(),
        retention_days: 30,
    })?;
    let environment_response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/console/environments")
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .header("content-type", "application/json")
                .body(Body::from(environment_body))?,
        )
        .await?;
    assert_eq!(environment_response.status(), StatusCode::CREATED);
    let environment_json: serde_json::Value =
        serde_json::from_slice(&environment_response.into_body().collect().await?.to_bytes())?;
    assert_eq!(environment_json["slug"], "production");
    assert_eq!(environment_json["retention_days"], 30);

    store
        .update_environment_retention(&session.raw, &fixture.environment, 90)
        .await
        .context("update retention")?;
    let retention: i32 =
        sqlx::query_scalar("SELECT retention_days FROM control.environments WHERE id = $1::uuid")
            .bind(&fixture.environment)
            .fetch_one(&admin)
            .await
            .context("read retention")?;
    assert_eq!(retention, 90);

    let source = store
        .create_data_source(
            &session.raw,
            CreateDataSourceRequest {
                project_id: fixture.project.clone(),
                environment_id: fixture.environment.clone(),
                name: "Rust integration".to_owned(),
                kind: "server".to_owned(),
            },
        )
        .await
        .context("create source")?;
    assert_eq!(source.status, "active");

    let create_key_body = serde_json::to_vec(&CreateSDKKeyRequest {
        project_id: fixture.project.clone(),
        environment_id: fixture.environment.clone(),
        data_source_id: source.id,
        name: "Rust SDK".to_owned(),
        scopes: vec!["ingest:otlp".to_owned(), "ingest:replay".to_owned()],
        expires_at: None,
    })?;
    let create_key_response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/console/sdk-keys")
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .header("content-type", "application/json")
                .body(Body::from(create_key_body))?,
        )
        .await?;
    assert_eq!(create_key_response.status(), StatusCode::CREATED);
    let created_json: serde_json::Value =
        serde_json::from_slice(&create_key_response.into_body().collect().await?.to_bytes())?;
    let original_id = created_json["key_id"]
        .as_str()
        .context("created key ID missing")?;
    let original_credential = created_json["credential"]
        .as_str()
        .context("created credential missing")?;
    assert!(parse_credential_prefix(original_credential, CredentialKind::SdkKey).is_ok());

    let rotate_response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/console/sdk-keys/{original_id}/rotate"))
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(rotate_response.status(), StatusCode::CREATED);
    let rotated_json: serde_json::Value =
        serde_json::from_slice(&rotate_response.into_body().collect().await?.to_bytes())?;
    let replacement_id = rotated_json["key_id"]
        .as_str()
        .context("replacement key ID missing")?
        .to_owned();
    let replacement_credential = rotated_json["credential"]
        .as_str()
        .context("replacement credential missing")?
        .to_owned();
    assert_ne!(replacement_id, original_id);
    let statuses: Vec<(String, String)> = sqlx::query_as(
        "SELECT id::text, status FROM control.sdk_keys WHERE id IN ($1::uuid, $2::uuid) ORDER BY status",
    )
    .bind(original_id)
    .bind(&replacement_id)
    .fetch_all(&admin)
    .await?;
    assert_eq!(statuses.len(), 2);
    assert!(
        statuses
            .iter()
            .any(|(id, status)| id == original_id && status == "revoked")
    );
    assert!(
        statuses
            .iter()
            .any(|(id, status)| id == &replacement_id && status == "active")
    );

    let sampling_body = serde_json::to_vec(&ActivateSamplingRequest {
        project_id: fixture.project.clone(),
        environment_id: fixture.environment.clone(),
        behavior_numerator: 1,
        behavior_denominator: 2,
        replay_numerator: 1,
        replay_denominator: 10,
        salt_version: "rust-v1".to_owned(),
    })?;
    let sampling_response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/console/sampling")
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .header("content-type", "application/json")
                .body(Body::from(sampling_body))?,
        )
        .await?;
    assert_eq!(sampling_response.status(), StatusCode::NO_CONTENT);
    let active_sampling: (i64, i64, i64) = sqlx::query_as(
        "SELECT version, behavior_numerator, replay_denominator FROM control.sampling_policies WHERE environment_id = $1::uuid AND status = 'active'",
    )
    .bind(&fixture.environment)
    .fetch_one(&admin)
    .await?;
    assert_eq!(active_sampling, (1, 1, 10));

    let schema_request = CreateSchemaRequest {
        project_id: fixture.project.clone(),
        version: "1.0.0".to_owned(),
        url: "https://schemas.chill.dev/test/rust-v1.json".to_owned(),
        definition: serde_json::json!({"type": "object", "properties": {}}),
        compatibility: "exact".to_owned(),
    };
    let schema_body = serde_json::to_vec(&schema_request)?;
    let schema_response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/console/schemas")
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .header("content-type", "application/json")
                .body(Body::from(schema_body.clone()))?,
        )
        .await?;
    assert_eq!(schema_response.status(), StatusCode::CREATED);
    let duplicate_schema = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/console/schemas")
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .header("content-type", "application/json")
                .body(Body::from(schema_body))?,
        )
        .await?;
    assert_eq!(duplicate_schema.status(), StatusCode::CONFLICT);

    let privacy_body = serde_json::to_vec(&ActivatePrivacyRequest {
        project_id: fixture.project.clone(),
        environment_id: fixture.environment.clone(),
        document: serde_json::json!({
            "schema_version": "1.0.0",
            "policy_version": "privacy-v1",
            "default_disposition": "omit",
            "annotation_allowlist": {"cat.id": "pseudonymous_identifier"},
            "source_allowlist": ["platform"],
            "replay": {
                "mask_at_source": true,
                "text": "mask",
                "form_values": "mask",
                "accessibility_text": "mask",
                "secure_input": "mask",
                "pixels": "mask",
                "custom_drawing": "block"
            }
        }),
    })?;
    let privacy_response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/console/privacy")
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .header("content-type", "application/json")
                .body(Body::from(privacy_body))?,
        )
        .await?;
    assert_eq!(privacy_response.status(), StatusCode::NO_CONTENT);

    let policy_request = SetCollectionPolicyRequest {
        project_id: fixture.project.clone(),
        environment_id: fixture.environment.clone(),
        expected_revision: 1,
        policy_version: "privacy-v1".to_owned(),
        enabled: true,
        disabled_capture_classes: vec!["replay".to_owned(), "analytics".to_owned()],
        idempotency_key: format!("rust-policy-{}", Uuid::new_v4()),
    };
    let policy = store
        .set_collection_policy(&session.raw, policy_request.clone())
        .await
        .context("replace collection policy")?;
    assert_eq!(policy.revision, 2);
    assert_eq!(
        policy.disabled_capture_classes,
        vec!["analytics".to_owned(), "replay".to_owned()]
    );
    let replayed = store
        .set_collection_policy(&session.raw, policy_request.clone())
        .await
        .context("replay collection policy")?;
    assert_eq!(replayed.revision, 2);
    let mut conflicting_policy = policy_request;
    conflicting_policy.enabled = false;
    assert!(matches!(
        store
            .set_collection_policy(&session.raw, conflicting_policy)
            .await,
        Err(ControlPlaneError::Conflict)
    ));

    let collection_response = collection_router(store.clone())
        .oneshot(
            Request::builder()
                .uri("/v1/chill/collection-state")
                .header(AUTHORIZATION, format!("Bearer {replacement_credential}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(collection_response.status(), StatusCode::OK);
    assert_eq!(
        collection_response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok()),
        Some("\"collection-2\"")
    );
    let collection_json: serde_json::Value =
        serde_json::from_slice(&collection_response.into_body().collect().await?.to_bytes())?;
    assert_eq!(collection_json["policy_version"], "privacy-v1");
    assert_eq!(collection_json["enabled"], true);
    assert_eq!(
        collection_json["disabled_capture_classes"],
        serde_json::json!(["analytics", "replay"])
    );

    let revoke_response = console_router(store.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/console/sdk-keys/{replacement_id}"))
                .header(AUTHORIZATION, format!("Bearer {}", session.raw))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(revoke_response.status(), StatusCode::NO_CONTENT);
    let replacement_status: String =
        sqlx::query_scalar("SELECT status FROM control.sdk_keys WHERE id = $1::uuid")
            .bind(&replacement_id)
            .fetch_one(&admin)
            .await?;
    assert_eq!(replacement_status, "revoked");

    let service = store
        .issue_service_credential(
            &session.raw,
            ServiceCredentialRequest {
                project_id: fixture.project.clone(),
                environment_id: fixture.environment.clone(),
                name: "Rust worker".to_owned(),
                scopes: vec![Capability::DataRead, Capability::ControlRead],
                expires_at: None,
            },
        )
        .await
        .context("issue service credential")?;
    let authenticated_service = store
        .authenticate_service_credential(&service.credential)
        .await
        .context("authenticate service credential")?;
    assert!(authenticated_service.allows(Capability::DataRead));
    assert!(!authenticated_service.allows(Capability::DataDelete));
    let rotated_service = store
        .rotate_service_credential(&session.raw, &service.credential_id)
        .await
        .context("rotate service credential")?;
    assert!(matches!(
        store
            .authenticate_service_credential(&service.credential)
            .await,
        Err(ControlPlaneError::Unauthorized)
    ));
    store
        .authenticate_service_credential(&rotated_service.credential)
        .await
        .context("authenticate rotated service credential")?;
    store
        .revoke_service_credential(&session.raw, &rotated_service.credential_id)
        .await
        .context("revoke service credential")?;
    assert!(matches!(
        store
            .authenticate_service_credential(&rotated_service.credential)
            .await,
        Err(ControlPlaneError::Unauthorized)
    ));

    let identity_session = store
        .issue_user_session_after_identity_verification(
            &fixture.organization,
            &VerifiedIdentity {
                issuer: "chill.email".to_owned(),
                subject: fixture.owner_email.clone(),
            },
            Duration::from_hours(1),
        )
        .await
        .context("issue identity session")?;
    let rotated_session = store
        .rotate_user_session(&identity_session.credential)
        .await
        .context("rotate identity session")?;
    assert_eq!(rotated_session.expires_at, identity_session.expires_at);
    assert!(matches!(
        store
            .authenticate_user_session(&identity_session.credential)
            .await,
        Err(ControlPlaneError::Unauthorized)
    ));
    store
        .revoke_user_session(&rotated_session.credential)
        .await
        .context("revoke rotated identity session")?;
    assert!(matches!(
        store
            .authenticate_user_session(&rotated_session.credential)
            .await,
        Err(ControlPlaneError::Unauthorized)
    ));

    let cross_tenant = store
        .update_environment_retention(&session.raw, &fixture.other_environment, 7)
        .await;
    assert!(matches!(cross_tenant, Err(ControlPlaneError::Forbidden)));
    Ok(())
}

struct Fixture {
    organization: String,
    project: String,
    environment: String,
    other_environment: String,
    owner_email: String,
}

async fn seed_fixture(admin: &PgPool, prefix: &str, digest: &[u8]) -> Result<Fixture> {
    let organization_id = Uuid::new_v4().to_string();
    let user_id = Uuid::new_v4().to_string();
    let project_id = Uuid::new_v4().to_string();
    let environment_id = Uuid::new_v4().to_string();
    let other_organization_id = Uuid::new_v4().to_string();
    let other_project_id = Uuid::new_v4().to_string();
    let other_environment_id = Uuid::new_v4().to_string();
    let slug_suffix = Uuid::new_v4().simple().to_string();
    let organization_slug = format!("rust-{}", &slug_suffix[..12]);
    let other_slug = format!("other-{}", &slug_suffix[12..24]);
    let owner_email = format!("{organization_slug}@example.invalid");

    let mut transaction = admin.begin().await.context("begin fixture")?;
    sqlx::query("INSERT INTO control.organizations (id, slug, name) VALUES ($1::uuid, $2, 'Rust Integration')")
        .bind(&organization_id).bind(&organization_slug).execute(&mut *transaction).await.context("seed organization")?;
    sqlx::query(
        "INSERT INTO control.users (id, email, display_name) VALUES ($1::uuid, $2, 'Rust Owner')",
    )
    .bind(&user_id)
    .bind(&owner_email)
    .execute(&mut *transaction)
    .await
    .context("seed user")?;
    sqlx::query(
        "INSERT INTO control.user_identities (user_id, issuer, subject) VALUES ($1::uuid, 'chill.email', $2)",
    )
    .bind(&user_id)
    .bind(&owner_email)
    .execute(&mut *transaction)
    .await
    .context("seed identity")?;
    sqlx::query("INSERT INTO control.organization_memberships (organization_id, user_id, role) VALUES ($1::uuid, $2::uuid, 'owner')")
        .bind(&organization_id).bind(&user_id).execute(&mut *transaction).await.context("seed membership")?;
    sqlx::query("INSERT INTO control.projects (id, organization_id, slug, name) VALUES ($1::uuid, $2::uuid, 'project', 'Project')")
        .bind(&project_id).bind(&organization_id).execute(&mut *transaction).await.context("seed project")?;
    sqlx::query("INSERT INTO control.environments (id, organization_id, project_id, slug, name, kind) VALUES ($1::uuid, $2::uuid, $3::uuid, 'development', 'Development', 'development')")
        .bind(&environment_id).bind(&organization_id).bind(&project_id).execute(&mut *transaction).await.context("seed environment")?;
    sqlx::query("INSERT INTO control.user_sessions (organization_id, user_id, prefix, secret_digest, expires_at) VALUES ($1::uuid, $2::uuid, $3, $4, clock_timestamp() + interval '1 hour')")
        .bind(&organization_id).bind(&user_id).bind(prefix).bind(digest).execute(&mut *transaction).await.context("seed session")?;

    sqlx::query(
        "INSERT INTO control.organizations (id, slug, name) VALUES ($1::uuid, $2, 'Other Tenant')",
    )
    .bind(&other_organization_id)
    .bind(&other_slug)
    .execute(&mut *transaction)
    .await
    .context("seed other organization")?;
    sqlx::query("INSERT INTO control.projects (id, organization_id, slug, name) VALUES ($1::uuid, $2::uuid, 'project', 'Other Project')")
        .bind(&other_project_id).bind(&other_organization_id).execute(&mut *transaction).await.context("seed other project")?;
    sqlx::query("INSERT INTO control.environments (id, organization_id, project_id, slug, name, kind) VALUES ($1::uuid, $2::uuid, $3::uuid, 'development', 'Other Development', 'development')")
        .bind(&other_environment_id).bind(&other_organization_id).bind(&other_project_id).execute(&mut *transaction).await.context("seed other environment")?;
    transaction.commit().await.context("commit fixture")?;

    Ok(Fixture {
        organization: organization_id,
        project: project_id,
        environment: environment_id,
        other_environment: other_environment_id,
        owner_email,
    })
}
