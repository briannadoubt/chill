//! `PostgreSQL`, object-cache, and embedded-engine query service coverage.

use std::{env, fs, sync::Arc, time::Duration};

use anyhow::{Context as _, Result};
use chill_control_plane::{BootstrapRequest, CredentialIssuer, Store};
use chill_lake::{Batch, Row, deterministic_batch_id, encode_manifest, encode_parquet};
use chill_objects::ImmutableStore;
use chill_query::{
    BehaviorFilter, Catalog, CatalogError, Engine, EngineConfiguration, EventsPlan, FileCache,
    Kind, PLAN_VERSION, Plan, ResultCache, Scope, Service, ServiceConfiguration, TimeRange,
};
use sha2::{Digest as _, Sha256};
use sqlx::{Executor as _, postgres::PgPoolOptions};
use time::{Date, Month};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires CHILL_TEST_DATABASE_URL and PostgreSQL"]
#[allow(
    clippy::too_many_lines,
    reason = "one production-shaped fixture verifies catalog, both caches, engine, and tenant isolation"
)]
async fn tenant_query_materializes_executes_and_caches() -> Result<()> {
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
        &format!("query-{suffix}@example.invalid"),
        serde_json::json!({"type":"object"}),
    );
    request.idempotency_key = format!("rust-query-bootstrap-{suffix}");
    request.organization.slug = format!("rust-query-{}", &suffix[..12]);
    request.project.slug = format!("project-{}", &suffix[12..24]);
    let bootstrap = Store::new(admin.clone(), issuer.clone())
        .bootstrap(request)
        .await?;
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
    let control = Store::new(runtime, issuer);
    let rows = behavior_rows(
        &bootstrap.organization_id,
        &bootstrap.project_id,
        &bootstrap.environment_id,
    );
    let record_digests: Vec<Vec<u8>> = rows
        .iter()
        .map(|row| Sha256::digest(row.canonical_json.as_bytes()).to_vec())
        .collect();
    let batch = Batch {
        id: deterministic_batch_id("micro", &bootstrap.environment_id, &record_digests, &[]),
        organization_id: bootstrap.organization_id.clone(),
        project_id: bootstrap.project_id.clone(),
        environment_id: bootstrap.environment_id.clone(),
        kind: "micro".to_owned(),
        partition_day: Date::from_calendar_date(1970, Month::January, 1)?,
        partition_hour: 0,
        envelope_kind: "behavior".to_owned(),
        row_count: rows.len(),
        min_server_received_at_unix_nano: 1,
        max_server_received_at_unix_nano: 1_000,
        min_effective_occurred_at_unix_nano: 1,
        max_effective_occurred_at_unix_nano: 1_000,
        attempt_count: 1,
        supersedes: Vec::new(),
    };
    let parquet = encode_parquet(&batch, &rows)?;
    let object_digest: [u8; 32] = Sha256::digest(&parquet).into();
    let object_key = batch.object_key()?;
    let manifest = encode_manifest(
        &batch,
        &object_key,
        object_digest,
        i64::try_from(parquet.len())?,
    )?;
    let manifest_digest: [u8; 32] = Sha256::digest(&manifest).into();
    let manifest_key = batch.manifest_key()?;
    let object_root = env::temp_dir().join(format!("chill-query-objects-{suffix}"));
    let objects = ImmutableStore::filesystem(&object_root)?;
    objects
        .put_if_absent(&object_key, parquet.clone().into(), object_digest)
        .await?;
    objects
        .put_if_absent(&manifest_key, manifest.into(), manifest_digest)
        .await?;
    sqlx::query(
        r"INSERT INTO lake.export_batches (batch_id,organization_id,project_id,environment_id,
          batch_kind,partition_day,partition_hour,envelope_kind,status,attempt_count,object_key,
          manifest_key,object_sha256,manifest_sha256,byte_count,row_count,
          min_server_received_at_unix_nano,max_server_received_at_unix_nano,
          min_effective_occurred_at_unix_nano,max_effective_occurred_at_unix_nano,published_at)
          VALUES ($1,$2::uuid,$3::uuid,$4::uuid,'micro',$5,0,'behavior','committed',1,$6,$7,$8,$9,$10,$11,
          1,1000,1,1000,clock_timestamp())",
    )
    .bind(&batch.id)
    .bind(&batch.organization_id)
    .bind(&batch.project_id)
    .bind(&batch.environment_id)
    .bind(batch.partition_day)
    .bind(&object_key)
    .bind(&manifest_key)
    .bind(object_digest.as_slice())
    .bind(manifest_digest.as_slice())
    .bind(i64::try_from(parquet.len())?)
    .bind(i32::try_from(rows.len())?)
    .execute(&admin)
    .await?;
    let cache_root = env::temp_dir().join(format!("chill-query-cache-{suffix}"));
    let files = FileCache::new(&cache_root, 16 << 20, objects)?;
    let engine = Arc::new(Engine::new(files.root(), EngineConfiguration::default())?);
    let service = Service::new(
        Catalog::new(control.clone()),
        files,
        ResultCache::new(1 << 20, 16, Duration::from_mins(1))?,
        engine,
        ServiceConfiguration::default(),
    )?;
    let scope = Scope {
        organization_id: bootstrap.organization_id.clone(),
        project_id: bootstrap.project_id.clone(),
        environment_id: bootstrap.environment_id.clone(),
    };
    let plan = events_plan();
    let first = service.execute(&scope, &plan).await?;
    assert_eq!(first.rows.len(), 2);
    assert!(!first.stats.cache_hit);
    assert_eq!(first.stats.file_count, 1);
    assert!(!first.stats.manifest_generation.is_empty());
    let second = service.execute(&scope, &plan).await?;
    assert!(second.stats.cache_hit);
    assert_eq!(second.rows.len(), 2);
    let mut wrong_scope = scope;
    wrong_scope.organization_id = "00000000-0000-4000-8000-000000000000".to_owned();
    assert!(matches!(
        Catalog::new(control).resolve(&wrong_scope, &plan).await,
        Err(CatalogError::ScopeNotFound)
    ));
    sqlx::query("DELETE FROM lake.export_batches WHERE organization_id=$1::uuid")
        .bind(&bootstrap.organization_id)
        .execute(&admin)
        .await?;
    let _ = fs::remove_dir_all(object_root);
    let _ = fs::remove_dir_all(cache_root);
    Ok(())
}

fn events_plan() -> Plan {
    Plan {
        version: PLAN_VERSION,
        kind: Kind::Events,
        range: TimeRange {
            start_unix_nano: 1,
            end_unix_nano: 1_000,
        },
        events: Some(EventsPlan {
            filter: BehaviorFilter::default(),
            session_id: String::new(),
            trace_id: String::new(),
            installation_id: String::new(),
            before: None,
            limit: 10,
        }),
        trace: None,
        replay: None,
        funnel: None,
        cohort: None,
        aggregate: None,
        path: None,
        retention: None,
    }
}

fn behavior_rows(organization: &str, project: &str, environment: &str) -> Vec<Row> {
    ["checkout", "purchase"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| Row {
            dataset_schema_version: String::new(),
            batch_id: String::new(),
            row_ordinal: 0,
            canonical_envelope_id: i64::try_from(index + 1).unwrap_or_default(),
            organization_id: organization.to_owned(),
            project_id: project.to_owned(),
            environment_id: environment.to_owned(),
            data_source_id: "44444444-4444-4444-8444-444444444444".to_owned(),
            envelope_version: "1.0.0".to_owned(),
            envelope_kind: "behavior".to_owned(),
            record_id: format!("record-{index}"),
            record_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            installation_id: None,
            session_id: None,
            replay_id: None,
            replay_chunk_id: None,
            trace_id: None,
            span_id: None,
            occurred_at_unix_nano: None,
            source_observed_at_unix_nano: None,
            server_received_at_unix_nano: u64::try_from(index + 100).unwrap_or_default(),
            effective_occurred_at_unix_nano: u64::try_from(index + 100).unwrap_or_default(),
            monotonic_nano: None,
            boot_id: None,
            sequence_number: None,
            clock_skew_nano: 0,
            timing_class: "on_time".to_owned(),
            late_arrival: false,
            canonical_json: format!(
                r#"{{"subject_id":"user-1","kind":"action","operation":"tap","name":"{name}"}}"#
            ),
            normalized_at_unix_nano: i64::try_from(index + 100).unwrap_or_default(),
        })
        .collect()
}
