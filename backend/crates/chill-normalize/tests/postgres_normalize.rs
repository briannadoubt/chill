//! `PostgreSQL` coverage for the durable Rust normalizer state machine.

use std::env;

use anyhow::{Context as _, Result};
use chill_control_plane::{BootstrapRequest, CredentialIssuer, Store};
use chill_ingest::{Candidate, Limits, PayloadFormat, Service, SignalKind};
use chill_normalize::{Decoder, Processor, ProcessorConfiguration, TimingPolicy};
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::InstrumentationScope,
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
};
use prost::Message as _;
use sqlx::{Executor as _, postgres::PgPoolOptions};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires CHILL_TEST_DATABASE_URL and PostgreSQL"]
async fn admitted_otlp_is_leased_canonicalized_and_completed() -> Result<()> {
    let database_url =
        env::var("CHILL_TEST_DATABASE_URL").context("CHILL_TEST_DATABASE_URL is required")?;
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;
    chill_migrations::migrate(&admin).await?;
    let issuer = CredentialIssuer::new(&[0x5c; 32])?;
    let suffix = Uuid::new_v4().simple().to_string();
    let mut bootstrap_request = BootstrapRequest::local_default(
        &format!("normalize-{suffix}@example.invalid"),
        serde_json::json!({"type":"object"}),
    );
    bootstrap_request.idempotency_key = format!("rust-normalize-bootstrap-{suffix}");
    bootstrap_request.organization.slug = format!("rust-normalize-{}", &suffix[..12]);
    bootstrap_request.project.slug = format!("project-{}", &suffix[12..24]);
    let bootstrap = Store::new(admin.clone(), issuer.clone())
        .bootstrap(bootstrap_request)
        .await?;
    let organization_id = bootstrap.organization_id.clone();
    let sdk_key = bootstrap.sdk_key.context("bootstrap SDK key missing")?;

    let runtime = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                connection.execute("SET ROLE chill_app").await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await?;
    let control = Store::new(runtime.clone(), issuer);
    let receipt = Service::new(control.clone(), Limits::default())?
        .accept(Candidate {
            kind: SignalKind::Logs,
            format: PayloadFormat::Protobuf,
            payload: generic_logs().encode_to_vec(),
            credential: sdk_key,
            idempotency_key: "rust-normalize-request".to_owned(),
            replay: None,
        })
        .await?;
    let processor = Processor::new(
        runtime,
        control.clone(),
        Decoder::new(TimingPolicy::default()),
        ProcessorConfiguration::production("rust-normalize-test"),
    )?;
    assert_eq!(processor.process_organization(&organization_id).await?, 1);

    let mut transaction = control.begin_tenant(&organization_id).await?;
    let status = sqlx::query_scalar::<_, String>("SELECT status FROM ingest.inbox WHERE id=$1")
        .bind(receipt.inbox_id)
        .fetch_one(&mut *transaction)
        .await?;
    let canonical_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM ingest.canonical_envelopes WHERE inbox_id=$1",
    )
    .bind(receipt.inbox_id)
    .fetch_one(&mut *transaction)
    .await?;
    assert_eq!(status, "completed");
    assert_eq!(canonical_count, 1);
    transaction.commit().await?;
    sqlx::query("DELETE FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid")
        .bind(&organization_id)
        .execute(&admin)
        .await?;
    sqlx::query("DELETE FROM ingest.inbox WHERE organization_id=$1::uuid")
        .bind(&organization_id)
        .execute(&admin)
        .await?;
    Ok(())
}

fn generic_logs() -> ExportLogsServiceRequest {
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                scope: Some(InstrumentationScope {
                    name: "example.instrumentation".to_owned(),
                    ..Default::default()
                }),
                log_records: vec![LogRecord {
                    time_unix_nano: 1_800_000_000_000_000_000,
                    event_name: "application.started".to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}
