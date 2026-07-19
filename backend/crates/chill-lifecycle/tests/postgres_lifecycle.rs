//! Restart-safe `PostgreSQL` deletion and immutable rewrite coverage.

use std::{
    env, fs,
    io::{Cursor, Read as _},
    time::Duration,
};

use anyhow::{Context as _, Result};
use chill_control_plane::{BootstrapRequest, CredentialIssuer, Store};
use chill_ingest::{Candidate, Limits, PayloadFormat, Service, SignalKind};
use chill_lake::{
    Processor as LakeProcessor, ProcessorConfiguration as LakeConfiguration, decode_parquet,
};
use chill_lifecycle::{
    Compatibility, Configuration, ExportConfiguration, ExportKind, ExportRequest, ExportTargetKind,
    Exporter, Kind, Manager, Request, SchemaField, SchemaRegistry, TargetKind,
    current_dataset_schema,
};
use chill_normalize::{
    Decoder, Processor as Normalizer, ProcessorConfiguration as NormalizeConfiguration,
    TimingPolicy,
};
use chill_objects::ImmutableStore;
use sha2::Digest as _;
use sqlx::{Executor as _, postgres::PgPoolOptions};
use time::OffsetDateTime;
use uuid::Uuid;

const ACTION_LOG: &str = include_str!("../../../../examples/otlp/v1/action-log.json");

#[tokio::test]
#[ignore = "requires CHILL_TEST_DATABASE_URL and PostgreSQL"]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end fixture verifies tombstone, restart, rewrite, purge, and audit invariants"
)]
async fn subject_deletion_recovers_a_lease_and_rewrites_immutable_lake_data() -> Result<()> {
    let database_url =
        env::var("CHILL_TEST_DATABASE_URL").context("CHILL_TEST_DATABASE_URL is required")?;
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;
    chill_migrations::migrate(&admin).await?;
    let mut proposed_schema = current_dataset_schema();
    proposed_schema.version += 1;
    proposed_schema.fields.push(SchemaField {
        name: "future_optional".to_owned(),
        logical_type: "string".to_owned(),
        required: false,
    });
    let proposal_digest = SchemaRegistry::new(admin.clone())
        .propose(proposed_schema, Compatibility::Full)
        .await?;
    let durable_digest: Vec<u8> = sqlx::query_scalar(
        "SELECT digest FROM lake.dataset_schemas WHERE version=2 AND status='draft'",
    )
    .fetch_one(&admin)
    .await?;
    assert_eq!(durable_digest, proposal_digest);
    let issuer = CredentialIssuer::new(&[0x5c; 32])?;
    let suffix = Uuid::new_v4().simple().to_string();
    let mut bootstrap_request = BootstrapRequest::local_default(
        &format!("lifecycle-{suffix}@example.invalid"),
        serde_json::json!({"type":"object"}),
    );
    bootstrap_request.idempotency_key = format!("rust-lifecycle-bootstrap-{suffix}");
    bootstrap_request.organization.slug = format!("rust-lifecycle-{}", &suffix[..12]);
    bootstrap_request.project.slug = format!("project-{}", &suffix[12..24]);
    let bootstrap = Store::new(admin.clone(), issuer.clone())
        .bootstrap(bootstrap_request)
        .await?;
    let sdk_key = bootstrap.sdk_key.context("bootstrap SDK key missing")?;
    let runtime = PgPoolOptions::new()
        .max_connections(6)
        .after_connect(|connection, _| {
            Box::pin(async move {
                connection.execute("SET ROLE chill_app").await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await?;
    let control = Store::new(runtime.clone(), issuer);
    let ingest = Service::new(control.clone(), Limits::default())?;
    let target_session = "0190f29f-ff00-7000-8000-000000000001";
    let survivor_session = "0190f29f-ff00-7000-8000-000000000002";
    ingest
        .accept(Candidate {
            kind: SignalKind::Logs,
            format: PayloadFormat::Json,
            payload: action_log(target_session, "0190f2a1-2b3c-7d4e-8f50-1234567890ab")?,
            credential: sdk_key.clone(),
            idempotency_key: "rust-lifecycle-target".to_owned(),
            replay: None,
        })
        .await?;
    ingest
        .accept(Candidate {
            kind: SignalKind::Logs,
            format: PayloadFormat::Json,
            payload: action_log(survivor_session, "0190f2a1-2b3c-7d4e-8f50-1234567890ac")?,
            credential: sdk_key,
            idempotency_key: "rust-lifecycle-survivor".to_owned(),
            replay: None,
        })
        .await?;
    let normalizer = Normalizer::new(
        runtime.clone(),
        control.clone(),
        Decoder::new(TimingPolicy::default()),
        NormalizeConfiguration::production("rust-lifecycle-normalizer"),
    )?;
    assert_eq!(
        normalizer
            .process_organization(&bootstrap.organization_id)
            .await?,
        2
    );
    let canonical_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid",
    )
    .bind(&bootstrap.organization_id)
    .fetch_one(&admin)
    .await?;
    if canonical_count != 2 {
        let inbox: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT status,last_error_code,last_error_message FROM ingest.inbox WHERE organization_id=$1::uuid ORDER BY server_received_at",
        )
        .bind(&bootstrap.organization_id)
        .fetch_all(&admin)
        .await?;
        anyhow::bail!("expected two canonical rows, found {canonical_count}; inbox={inbox:?}");
    }
    let root = env::temp_dir().join(format!("chill-lifecycle-{suffix}"));
    let objects = ImmutableStore::filesystem(&root)?;
    let lake = LakeProcessor::new(
        runtime.clone(),
        control.clone(),
        objects.clone(),
        LakeConfiguration::production("rust-lifecycle-lake"),
    )?;
    assert_eq!(
        lake.process_organization(&bootstrap.organization_id)
            .await?,
        1
    );
    let (source_key, source_digest): (String, Vec<u8>) = sqlx::query_as(
        "SELECT object_key,object_sha256 FROM lake.export_batches WHERE organization_id=$1::uuid AND status='committed'",
    )
    .bind(&bootstrap.organization_id)
    .fetch_one(&admin)
    .await?;
    let query_cache = root.join("query-cache");
    fs::create_dir_all(&query_cache)?;
    let cached_source = query_cache.join(format!("{}.parquet", hex::encode(&source_digest)));
    fs::write(&cached_source, objects.get(&source_key).await?)?;
    let manager = Manager::new(
        runtime,
        control.clone(),
        objects.clone(),
        Configuration {
            lease_duration: Duration::from_millis(20),
            retry_delay: Duration::from_millis(10),
            query_cache_path: query_cache,
            ..Configuration::production("rust-lifecycle-worker")
        },
    )?;
    let request = manager
        .create_request(Request {
            id: String::new(),
            organization_id: bootstrap.organization_id.clone(),
            project_id: bootstrap.project_id.clone(),
            environment_id: bootstrap.environment_id.clone(),
            kind: Kind::DataSubject,
            target_kind: Some(TargetKind::SessionId),
            target_value: target_session.to_owned(),
            target_sha256: [0; 32],
            cutoff_unix_nano: None,
            requested_by: "privacy.test".to_owned(),
            reason_code: "privacy.subject_request".to_owned(),
            idempotency_key: format!("subject-{suffix}"),
            attempt_count: 0,
            created_at: None,
        })
        .await?;
    let tombstones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM lifecycle.subject_tombstones WHERE request_id=$1::uuid",
    )
    .bind(&request.id)
    .fetch_one(&admin)
    .await?;
    assert_eq!(tombstones, 1);
    sqlx::query(
        r"UPDATE lifecycle.deletion_requests SET status='leased',lease_owner='crashed-worker',
          lease_expires_at=clock_timestamp()-interval '1 second',attempt_count=1 WHERE id=$1::uuid",
    )
    .bind(&request.id)
    .execute(&admin)
    .await?;
    assert_eq!(manager.process_available().await?, 1);
    let completion = sqlx::query_as::<_, (String, i64, i32, i32, Option<String>)>(
        r"SELECT status,deleted_rows,rewritten_files,deleted_files,target_value
          FROM lifecycle.deletion_requests WHERE id=$1::uuid",
    )
    .bind(&request.id)
    .fetch_one(&admin)
    .await?;
    assert_eq!(completion.0, "completed");
    assert_eq!(completion.1, 1);
    assert_eq!(completion.2, 1);
    assert_eq!(completion.3, 1);
    assert!(completion.4.is_none());
    assert!(objects.get(&source_key).await.is_err());
    assert!(!cached_source.exists());
    let replacement_key: String = sqlx::query_scalar(
        r"SELECT object_key FROM lake.export_batches WHERE organization_id=$1::uuid
          AND batch_kind='rewrite' AND status='committed'",
    )
    .bind(&bootstrap.organization_id)
    .fetch_one(&admin)
    .await?;
    let replacement = decode_parquet(objects.get(&replacement_key).await?)?;
    assert_eq!(replacement.len(), 1);
    assert_eq!(replacement[0].session_id.as_deref(), Some(survivor_session));
    let generation: i64 = sqlx::query_scalar(
        "SELECT generation FROM lifecycle.environment_generations WHERE environment_id=$1::uuid",
    )
    .bind(&bootstrap.environment_id)
    .fetch_one(&admin)
    .await?;
    assert_eq!(generation, 1);
    let exporter = Exporter::new(control, objects.clone(), ExportConfiguration::default())?;
    let export_request = ExportRequest {
        organization_id: bootstrap.organization_id.clone(),
        project_id: bootstrap.project_id.clone(),
        environment_id: bootstrap.environment_id.clone(),
        kind: ExportKind::DataSubject,
        target_kind: Some(ExportTargetKind::SessionId),
        target_value: survivor_session.to_owned(),
        requested_by: "privacy.test".to_owned(),
        reason_code: "privacy.subject_export".to_owned(),
        idempotency_key: format!("export-{suffix}"),
    };
    let mut delivered = Vec::new();
    let receipt = exporter
        .run(export_request.clone(), |artifact| {
            delivered = artifact.body;
            Ok::<(), std::io::Error>(())
        })
        .await?;
    assert_eq!(receipt.record_count, 1);
    assert_eq!(receipt.source_file_count, 1);
    assert_eq!(
        receipt.artifact_sha256,
        hex::encode(sha2::Sha256::digest(&delivered))
    );
    let mut archive = zip::ZipArchive::new(Cursor::new(delivered))?;
    let mut manifest = String::new();
    archive
        .by_name("manifest.json")?
        .read_to_string(&mut manifest)?;
    assert!(!manifest.contains(survivor_session));
    let mut records = String::new();
    archive
        .by_name("records.ndjson")?
        .read_to_string(&mut records)?;
    assert_eq!(records.lines().count(), 1);
    assert!(records.contains(survivor_session));
    let export_status: String =
        sqlx::query_scalar("SELECT status FROM compliance.export_requests WHERE id=$1::uuid")
            .bind(&receipt.request_id)
            .fetch_one(&admin)
            .await?;
    assert_eq!(export_status, "completed");

    let failed_key = format!("export-failed-{suffix}");
    let failed_result = exporter
        .run(
            ExportRequest {
                idempotency_key: failed_key.clone(),
                ..export_request
            },
            |_| Err(std::io::Error::other("delivery refused")),
        )
        .await;
    assert!(failed_result.is_err());
    let failed_status: String = sqlx::query_scalar(
        "SELECT status FROM compliance.export_requests WHERE organization_id=$1::uuid AND idempotency_key=$2",
    )
    .bind(&bootstrap.organization_id)
    .bind(failed_key)
    .fetch_one(&admin)
    .await?;
    assert_eq!(failed_status, "failed");
    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn action_log(session_id: &str, record_id: &str) -> Result<Vec<u8>> {
    let mut document: serde_json::Value = serde_json::from_str(ACTION_LOG)?;
    let record = &mut document["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];
    let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos())?;
    record["timeUnixNano"] = serde_json::json!(now.to_string());
    record["observedTimeUnixNano"] = serde_json::json!(now.saturating_add(1).to_string());
    let attributes = record["attributes"]
        .as_array_mut()
        .context("action-log attributes missing")?;
    attributes.retain(|attribute| {
        !matches!(
            attribute["key"].as_str(),
            Some(
                "chill.annotation.experiment.variant"
                    | "chill.privacy.annotation_classification.experiment.variant"
                    | "chill.annotation.cat.id"
                    | "chill.privacy.annotation_classification.cat.id"
                    | "chill.tenant.environment_id"
                    | "chill.tenant.organization_id"
                    | "chill.tenant.project_id"
            )
        )
    });
    for attribute in attributes {
        match attribute["key"].as_str() {
            Some("chill.context.session_id" | "session.id") => {
                attribute["value"]["stringValue"] = serde_json::json!(session_id);
            }
            Some("chill.record.id" | "chill.subject.id") => {
                attribute["value"]["stringValue"] = serde_json::json!(record_id);
            }
            Some("chill.privacy.policy_version") => {
                attribute["value"]["stringValue"] = serde_json::json!("privacy-v1");
            }
            _ => {}
        }
    }
    Ok(serde_json::to_vec(&document)?)
}
