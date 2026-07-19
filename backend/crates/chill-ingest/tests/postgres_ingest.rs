//! `PostgreSQL` integration coverage for authenticated durable Rust ingestion.

use std::env;

use anyhow::{Context as _, Result};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chill_control_plane::{BootstrapRequest, CredentialIssuer, Store};
use chill_ingest::{
    Candidate, ErrorCode, Limits, PayloadFormat, Service, SignalKind, grpc_router, ingest_router,
};
use opentelemetry_proto::tonic::{
    collector::logs::v1::{ExportLogsServiceRequest, logs_service_client::LogsServiceClient},
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
};
use prost::Message as _;
use sha2::{Digest as _, Sha256};
use sqlx::{Executor as _, postgres::PgPoolOptions};
use tower::ServiceExt as _;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires CHILL_TEST_DATABASE_URL and PostgreSQL"]
async fn authenticated_requests_are_idempotently_durable() -> Result<()> {
    let database_url =
        env::var("CHILL_TEST_DATABASE_URL").context("CHILL_TEST_DATABASE_URL is required")?;
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;
    chill_migrations::migrate(&admin).await?;
    let issuer = CredentialIssuer::new(&[0x5c; 32])?;
    let suffix = Uuid::new_v4().simple().to_string();
    let mut request = BootstrapRequest::local_default(
        &format!("ingest-{suffix}@example.invalid"),
        serde_json::json!({"type":"object"}),
    );
    request.idempotency_key = format!("rust-ingest-bootstrap-{suffix}");
    request.organization.slug = format!("rust-ingest-{}", &suffix[..12]);
    request.project.slug = format!("project-{}", &suffix[12..24]);
    let bootstrap = Store::new(admin, issuer.clone()).bootstrap(request).await?;
    let sdk_key = bootstrap.sdk_key.context("bootstrap SDK key missing")?;

    let runtime = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                connection.execute("SET ROLE chill_app").await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await?;
    let service = Service::new(Store::new(runtime, issuer), Limits::default())?;
    let candidate = logs_candidate(&sdk_key, "rust-ingest-request", "record-one");
    let first = service.accept(candidate).await?;
    assert!(!first.duplicate);
    let duplicate = service
        .accept(logs_candidate(
            &sdk_key,
            "rust-ingest-request",
            "record-one",
        ))
        .await?;
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.inbox_id, first.inbox_id);
    let conflict = service
        .accept(logs_candidate(
            &sdk_key,
            "rust-ingest-request",
            "record-two",
        ))
        .await;
    assert_eq!(
        conflict.err().map(|error| error.code),
        Some(ErrorCode::Conflict)
    );
    let unauthorized = service
        .accept(logs_candidate(
            &format!("{sdk_key}x"),
            "rust-ingest-unauthorized",
            "record-three",
        ))
        .await;
    assert_eq!(
        unauthorized.err().map(|error| error.code),
        Some(ErrorCode::Unauthorized)
    );

    let replay_payload = b"CHILLRP1\nabcd";
    let replay_response = ingest_router(service.clone(), Limits::default())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chill/replay")
                .header(
                    "content-type",
                    "application/vnd.chill.replay.v1+octet-stream",
                )
                .header("authorization", format!("Bearer {sdk_key}"))
                .header("chill-replay-id", "replay-http")
                .header("chill-replay-chunk-id", "chunk-http")
                .header("chill-replay-session-id", "session-http")
                .header("chill-replay-boot-id", "boot-http")
                .header("chill-replay-start-monotonic-nano", "1")
                .header("chill-replay-end-monotonic-nano", "2")
                .header("chill-replay-occurred-at-unix-nano", "3")
                .header(
                    "chill-replay-digest",
                    hex::encode(Sha256::digest(replay_payload)),
                )
                .header("chill-replay-codec", "chill-json-gzip-aesgcm-v1")
                .body(Body::from(replay_payload.as_slice()))?,
        )
        .await?;
    assert_eq!(replay_response.status(), StatusCode::OK);
    assert_eq!(
        replay_response
            .headers()
            .get("idempotency-key")
            .and_then(|value| value.to_str().ok()),
        Some("replay:chunk-http")
    );

    verify_grpc_transport(service, &sdk_key).await?;
    Ok(())
}

async fn verify_grpc_transport(service: Service, sdk_key: &str) -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move { axum::serve(listener, grpc_router(service)).await });
    let mut client = LogsServiceClient::connect(format!("http://{address}")).await?;
    let mut request = tonic::Request::new(logs_request("record-grpc"));
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {sdk_key}").parse()?);
    request
        .metadata_mut()
        .insert("idempotency-key", "rust-ingest-grpc".parse()?);
    let response = client.export(request).await?;
    assert_eq!(
        response
            .metadata()
            .get("idempotency-key")
            .and_then(|value| value.to_str().ok()),
        Some("rust-ingest-grpc")
    );
    assert!(response.metadata().get("chill-inbox-id").is_some());
    server.abort();
    Ok(())
}

fn logs_candidate(credential: &str, idempotency_key: &str, record_id: &str) -> Candidate {
    Candidate {
        kind: SignalKind::Logs,
        format: PayloadFormat::Protobuf,
        payload: logs_request(record_id).encode_to_vec(),
        credential: credential.to_owned(),
        idempotency_key: idempotency_key.to_owned(),
        replay: None,
    }
}

fn logs_request(record_id: &str) -> ExportLogsServiceRequest {
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                scope: Some(InstrumentationScope {
                    name: "dev.chill.swift".to_owned(),
                    ..Default::default()
                }),
                log_records: vec![LogRecord {
                    event_name: "test".to_owned(),
                    attributes: vec![
                        string_attribute("chill.record.id", record_id),
                        string_attribute("chill.schema.version", "1.0.0"),
                        string_attribute(
                            "chill.schema.url",
                            "https://schemas.chill.dev/behavior/v1/envelope.schema.json",
                        ),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn string_attribute(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_owned(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.to_owned())),
        }),
        ..Default::default()
    }
}
