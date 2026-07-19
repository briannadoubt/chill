//! `PostgreSQL` and filesystem coverage for Rust micro-batch publication.

use std::env;

use anyhow::{Context as _, Result};
use chill_control_plane::{BootstrapRequest, CredentialIssuer, Store};
use chill_ingest::{Candidate, Limits, PayloadFormat, Service, SignalKind};
use chill_lake::{Manifest, Processor, ProcessorConfiguration};
use chill_normalize::{
    Decoder, Processor as Normalizer, ProcessorConfiguration as NormalizeConfiguration,
    TimingPolicy,
};
use chill_objects::ImmutableStore;
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::InstrumentationScope,
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use prost::Message as _;
use sqlx::{Executor as _, postgres::PgPoolOptions};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires CHILL_TEST_DATABASE_URL and PostgreSQL"]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end fixture verifies publish, recovery, and compaction state together"
)]
async fn canonical_rows_publish_as_immutable_parquet_and_manifest() -> Result<()> {
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
        &format!("lake-{suffix}@example.invalid"),
        serde_json::json!({"type":"object"}),
    );
    bootstrap_request.idempotency_key = format!("rust-lake-bootstrap-{suffix}");
    bootstrap_request.organization.slug = format!("rust-lake-{}", &suffix[..12]);
    bootstrap_request.project.slug = format!("project-{}", &suffix[12..24]);
    let bootstrap = Store::new(admin.clone(), issuer.clone())
        .bootstrap(bootstrap_request)
        .await?;
    let organization_id = bootstrap.organization_id.clone();
    let sdk_key = bootstrap.sdk_key.context("bootstrap SDK key missing")?;
    let runtime = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                connection.execute("SET ROLE chill_app").await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await?;
    let control = Store::new(runtime.clone(), issuer);
    Service::new(control.clone(), Limits::default())?
        .accept(Candidate {
            kind: SignalKind::Logs,
            format: PayloadFormat::Protobuf,
            payload: generic_logs(4).encode_to_vec(),
            credential: sdk_key,
            idempotency_key: "rust-lake-request".to_owned(),
            replay: None,
        })
        .await?;
    Normalizer::new(
        runtime.clone(),
        control.clone(),
        Decoder::new(TimingPolicy::default()),
        NormalizeConfiguration::production("rust-lake-normalizer"),
    )?
    .process_organization(&organization_id)
    .await?;

    let root = env::temp_dir().join(format!("chill-lake-{}", Uuid::new_v4()));
    let objects = ImmutableStore::filesystem(&root)?;
    let mut configuration = ProcessorConfiguration::production("rust-lake-publisher");
    configuration.batch_size = 1;
    configuration.compaction_minimum = 2;
    configuration.compaction_maximum = 4;
    configuration.maximum_compacted_rows = 8;
    let processor = Processor::new(runtime, control.clone(), objects.clone(), configuration)?;
    assert_eq!(processor.process_organization(&organization_id).await?, 1);
    // Force the first committed batch back through the durable pending path. The
    // immutable write must be accepted as an identical retry and the aliased
    // numeric state must reconstruct the original batch exactly.
    let mut recovery = control.begin_tenant(&organization_id).await?;
    sqlx::query(
        r"UPDATE lake.export_batches SET status='pending',available_at=clock_timestamp(),
          lease_owner=NULL,lease_expires_at=NULL,object_key=NULL,manifest_key=NULL,
          object_sha256=NULL,manifest_sha256=NULL,byte_count=NULL,published_at=NULL
          WHERE organization_id=$1::uuid AND batch_kind='micro'",
    )
    .bind(&organization_id)
    .execute(&mut *recovery)
    .await?;
    recovery.commit().await?;
    assert_eq!(processor.process_organization(&organization_id).await?, 1);
    for _ in 0..3 {
        assert_eq!(processor.process_organization(&organization_id).await?, 1);
    }
    // With no unclaimed canonical rows left, the next pass atomically compacts
    // the four one-row objects and supersedes their committed source set.
    assert_eq!(processor.process_organization(&organization_id).await?, 1);
    let mut transaction = control.begin_tenant(&organization_id).await?;
    let (status, object_key, manifest_key, attempt_count) =
        sqlx::query_as::<_, (String, String, String, i32)>(
            r"SELECT status,object_key,manifest_key,attempt_count FROM lake.export_batches
              WHERE organization_id=$1::uuid AND batch_kind='compaction'",
        )
        .bind(&organization_id)
        .fetch_one(&mut *transaction)
        .await?;
    assert_eq!(status, "committed");
    assert_eq!(attempt_count, 1);
    let (superseded, retried) = sqlx::query_as::<_, (i64, i64)>(
        r"SELECT count(*) FILTER (WHERE status='superseded'),
          count(*) FILTER (WHERE batch_kind='micro' AND attempt_count=2)
          FROM lake.export_batches WHERE organization_id=$1::uuid",
    )
    .bind(&organization_id)
    .fetch_one(&mut *transaction)
    .await?;
    assert_eq!(superseded, 4);
    assert_eq!(retried, 1);
    transaction.commit().await?;
    let manifest: Manifest = serde_json::from_slice(&objects.get(&manifest_key).await?)?;
    assert_eq!(manifest.row_count, 4);
    let parquet = objects.get(&object_key).await?;
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(parquet)
        .and_then(ParquetRecordBatchReaderBuilder::build)?;
    assert_eq!(
        reader
            .next()
            .transpose()?
            .context("missing Parquet batch")?
            .num_rows(),
        4
    );
    sqlx::query("DELETE FROM lake.batch_records WHERE organization_id=$1::uuid")
        .bind(&organization_id)
        .execute(&admin)
        .await?;
    sqlx::query("DELETE FROM lake.source_claims WHERE organization_id=$1::uuid")
        .bind(&organization_id)
        .execute(&admin)
        .await?;
    sqlx::query("DELETE FROM lake.compaction_sources WHERE organization_id=$1::uuid")
        .bind(&organization_id)
        .execute(&admin)
        .await?;
    sqlx::query("DELETE FROM lake.export_batches WHERE organization_id=$1::uuid")
        .bind(&organization_id)
        .execute(&admin)
        .await?;
    sqlx::query("DELETE FROM ingest.canonical_envelopes WHERE organization_id=$1::uuid")
        .bind(&organization_id)
        .execute(&admin)
        .await?;
    sqlx::query("DELETE FROM ingest.inbox WHERE organization_id=$1::uuid")
        .bind(&organization_id)
        .execute(&admin)
        .await?;
    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

fn generic_logs(count: usize) -> ExportLogsServiceRequest {
    let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos()).unwrap_or_default();
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                scope: Some(InstrumentationScope {
                    name: "example.instrumentation".to_owned(),
                    ..Default::default()
                }),
                log_records: (0..count)
                    .map(|index| LogRecord {
                        time_unix_nano: now,
                        event_name: format!("application.event.{index}"),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}
