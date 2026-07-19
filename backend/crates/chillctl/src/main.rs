//! Administrative command-line interface for the Rust Chill backend.

use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chill_control_plane::{BootstrapRequest, CredentialIssuer, Store, VerifiedIdentity};
use chill_lake::{DATASET_SCHEMA_VERSION, Manifest, decode_parquet};
use chill_lifecycle::{
    Compatibility, Configuration as LifecycleConfiguration, DatasetSchema, ExportConfiguration,
    ExportKind, ExportRequest, ExportTargetKind, Exporter, Kind as LifecycleKind,
    Manager as LifecycleManager, Request as LifecycleRequest, SchemaRegistry, TargetKind,
};
use chill_objects::ImmutableStore;
use chill_query::{
    Catalog, Engine, EngineConfiguration, FileCache, Plan, ResultCache, Scope,
    Service as QueryService, ServiceConfiguration,
};
use clap::{Args as ClapArgs, Parser, Subcommand};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;

type LakeLedgerRow = (String, String, String, Vec<u8>, Vec<u8>, i64, i32);

#[derive(Debug, Parser)]
#[command(name = "chillctl", about = "Administrative CLI for Chill")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, ClapArgs)]
struct IssueVerifiedSessionArguments {
    /// Organization UUID returned by bootstrap.
    #[arg(long)]
    organization: String,
    /// Stable verified identity-provider namespace.
    #[arg(long)]
    issuer: String,
    /// Stable verified identity-provider subject.
    #[arg(long)]
    subject: String,
    /// Session lifetime in minutes (5 through 43,200).
    #[arg(long, default_value_t = 480)]
    lifetime_minutes: u64,
    /// Acknowledge that the identity assertion was verified outside Chill.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    identity_verified: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Verify that the local server is ready.
    Health,
    /// Print the local server's Prometheus metrics.
    Metrics,
    /// Create and clear the private container query-cache directory.
    InitQueryCache,
    /// Apply immutable `PostgreSQL` migrations.
    Migrate,
    /// Apply migrations and create or rotate the restricted runtime role.
    MigrateAndProvision,
    /// Create the complete first tenant idempotently.
    Bootstrap {
        /// Strict JSON bootstrap request.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Canonical behavior JSON Schema used by the default request.
        #[arg(long)]
        schema: Option<PathBuf>,
        /// Owner email used by the default request.
        #[arg(long)]
        owner_email: Option<String>,
    },
    /// Issue a one-time user session for an identity verified by a trusted operator.
    IssueVerifiedSession(IssueVerifiedSessionArguments),
    /// Verify restored control-plane and compliance invariants.
    Verify {
        /// Minimum expected organization rows.
        #[arg(long, default_value_t = 0)]
        minimum_organizations: i64,
    },
    /// Download and verify committed lake objects and manifests.
    VerifyLake {
        /// Maximum committed files to inspect.
        #[arg(long, default_value_t = 1_000)]
        maximum_files: usize,
    },
    /// Execute one typed query plan.
    Query {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        organization: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        cache_path: Option<PathBuf>,
    },
    /// Build and atomically deliver one privacy export.
    PrivacyExport {
        #[arg(long)]
        kind: String,
        #[arg(long)]
        organization: String,
        #[arg(long, default_value = "")]
        project: String,
        #[arg(long, default_value = "")]
        environment: String,
        #[arg(long)]
        target_kind: Option<String>,
        #[arg(long)]
        target_file: Option<PathBuf>,
        #[arg(long)]
        requested_by: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        idempotency_key: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Create one durable lifecycle request.
    LifecycleRequest {
        #[arg(long)]
        kind: String,
        #[arg(long)]
        organization: String,
        #[arg(long, default_value = "")]
        project: String,
        #[arg(long, default_value = "")]
        environment: String,
        #[arg(long)]
        target_kind: Option<String>,
        #[arg(long)]
        target_file: Option<PathBuf>,
        #[arg(long)]
        cutoff: Option<String>,
        #[arg(long)]
        requested_by: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        idempotency_key: String,
        #[arg(long)]
        cache_path: Option<PathBuf>,
    },
    /// Schedule retention and drain bounded lifecycle work.
    LifecycleRun {
        #[arg(long, default_value_t = 100)]
        maximum_requests: usize,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        schedule_retention: bool,
        #[arg(long)]
        cache_path: Option<PathBuf>,
    },
    /// Validate and store an editable dataset schema draft.
    SchemaPropose {
        #[arg(long)]
        definition: PathBuf,
        #[arg(long, default_value = "full")]
        compatibility: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    match arguments.command {
        Command::Health => health().await,
        Command::Metrics => metrics().await,
        Command::InitQueryCache => init_query_cache(),
        Command::Migrate => migrate().await,
        Command::MigrateAndProvision => migrate_and_provision().await,
        Command::Bootstrap {
            config,
            schema,
            owner_email,
        } => bootstrap(config, schema, owner_email).await,
        Command::IssueVerifiedSession(arguments) => issue_verified_session(arguments).await,
        Command::Verify {
            minimum_organizations,
        } => verify(minimum_organizations).await,
        Command::VerifyLake { maximum_files } => verify_lake(maximum_files).await,
        Command::Query {
            plan,
            organization,
            project,
            environment,
            cache_path,
        } => execute_query(plan, organization, project, environment, cache_path).await,
        Command::PrivacyExport {
            kind,
            organization,
            project,
            environment,
            target_kind,
            target_file,
            requested_by,
            reason,
            idempotency_key,
            output,
        } => {
            privacy_export(
                kind,
                organization,
                project,
                environment,
                target_kind,
                target_file,
                requested_by,
                reason,
                idempotency_key,
                output,
            )
            .await
        }
        Command::LifecycleRequest {
            kind,
            organization,
            project,
            environment,
            target_kind,
            target_file,
            cutoff,
            requested_by,
            reason,
            idempotency_key,
            cache_path,
        } => {
            lifecycle_request(
                kind,
                organization,
                project,
                environment,
                target_kind,
                target_file,
                cutoff,
                requested_by,
                reason,
                idempotency_key,
                cache_path,
            )
            .await
        }
        Command::LifecycleRun {
            maximum_requests,
            schedule_retention,
            cache_path,
        } => lifecycle_run(maximum_requests, schedule_retention, cache_path).await,
        Command::SchemaPropose {
            definition,
            compatibility,
        } => schema_propose(definition, compatibility).await,
    }
}

async fn health() -> Result<()> {
    let response = local_client()
        .get("http://127.0.0.1:4318/readyz")
        .send()
        .await
        .context("health request failed")?;
    if response.status() != reqwest::StatusCode::NO_CONTENT {
        bail!("health request returned HTTP {}", response.status());
    }
    Ok(())
}

async fn metrics() -> Result<()> {
    let response = local_client()
        .get("http://127.0.0.1:4318/metrics")
        .send()
        .await
        .context("metrics request failed")?
        .error_for_status()
        .context("metrics endpoint failed")?;
    let body = response
        .text()
        .await
        .context("reading metrics response failed")?;
    if body.len() > 1 << 20 {
        bail!("metrics response exceeds 1 MiB");
    }
    print!("{body}");
    Ok(())
}

fn init_query_cache() -> Result<()> {
    let path = Path::new("/var/lib/chill/query-cache");
    if !path.is_absolute() || path == Path::new("/") {
        bail!("storage path must be a non-root absolute path");
    }
    fs::create_dir_all(path).context("create query cache")?;
    let metadata = fs::symlink_metadata(path).context("inspect query cache")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("query cache must be a directory, not a symbolic link");
    }
    for entry in fs::read_dir(path).context("list query cache")? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry_path)?;
        } else {
            fs::remove_file(entry_path)?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt as _, chown};
        chown(path, Some(65_532), Some(65_532)).context("own query cache")?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

async fn migrate_and_provision() -> Result<()> {
    let pool = database_pool().await?;
    apply_migrations(&pool).await?;
    let password = required_environment("CHILL_RUNTIME_DATABASE_PASSWORD", 128)?;
    if !(32..=128).contains(&password.len())
        || !password.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        bail!("CHILL_RUNTIME_DATABASE_PASSWORD must be 32-128 ASCII letters or digits");
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='chill_runtime')")
            .fetch_one(&pool)
            .await?;
    let statement = if exists {
        format!(
            "ALTER ROLE chill_runtime WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS PASSWORD '{password}'"
        )
    } else {
        format!(
            "CREATE ROLE chill_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS PASSWORD '{password}'"
        )
    };
    // The only interpolation is restricted to ASCII alphanumerics above.
    sqlx::query(sqlx::AssertSqlSafe(statement.as_str()))
        .execute(&pool)
        .await?;
    for statement in [
        "GRANT chill_app TO chill_runtime WITH INHERIT TRUE, SET FALSE",
        "ALTER ROLE chill_runtime SET statement_timeout = '60s'",
        "ALTER ROLE chill_runtime SET idle_in_transaction_session_timeout = '15s'",
    ] {
        sqlx::query(statement).execute(&pool).await?;
    }
    print_json(&serde_json::json!({ "role": "chill_runtime", "provisioned": true }))
}

async fn migrate() -> Result<()> {
    let pool = database_pool().await?;
    let applied = apply_migrations(&pool).await?;
    println!(
        "{}",
        serde_json::json!({
            "applied": applied.iter().map(|migration| migration.name).collect::<Vec<_>>(),
            "count": applied.len(),
        })
    );
    Ok(())
}

async fn bootstrap(
    config: Option<PathBuf>,
    schema: Option<PathBuf>,
    owner_email: Option<String>,
) -> Result<()> {
    let pool = database_pool().await?;
    apply_migrations(&pool)
        .await
        .context("apply migrations before bootstrap")?;
    let request = if let Some(path) = config {
        read_json::<BootstrapRequest>(&path, 1 << 20)
            .with_context(|| format!("read bootstrap config {}", path.display()))?
    } else {
        let owner_email = owner_email
            .or_else(|| env::var("CHILL_OWNER_EMAIL").ok())
            .context("--owner-email or CHILL_OWNER_EMAIL is required without --config")?;
        let (definition, path) = load_behavior_schema(schema)?;
        eprintln!("Using canonical behavior schema {}.", path.display());
        BootstrapRequest::local_default(&owner_email, definition)
    };
    let pepper = decode_pepper(&required_environment("CHILL_KEY_PEPPER", 4096)?)?;
    let result = Store::new(
        pool,
        CredentialIssuer::new(&pepper).context("validate CHILL_KEY_PEPPER")?,
    )
    .bootstrap(request)
    .await
    .context("bootstrap tenant")?;
    if result.created {
        eprintln!(
            "The SDK key below is shown once. Store it in a secrets manager; Chill cannot recover it."
        );
    } else {
        eprintln!("Bootstrap already existed; no SDK key was revealed.");
    }
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

async fn issue_verified_session(arguments: IssueVerifiedSessionArguments) -> Result<()> {
    let IssueVerifiedSessionArguments {
        organization,
        issuer,
        subject,
        lifetime_minutes,
        identity_verified,
    } = arguments;
    if !identity_verified {
        bail!(
            "refusing to issue a session without --identity-verified; verify the upstream identity assertion first"
        );
    }
    if !(5..=43_200).contains(&lifetime_minutes) {
        bail!("session lifetime must be between 5 and 43,200 minutes");
    }
    let lifetime_seconds = lifetime_minutes
        .checked_mul(60)
        .context("session lifetime overflow")?;
    let pool = database_pool().await?;
    let pepper = decode_pepper(&required_environment("CHILL_KEY_PEPPER", 4096)?)?;
    let session = Store::new(
        pool,
        CredentialIssuer::new(&pepper).context("validate CHILL_KEY_PEPPER")?,
    )
    .issue_user_session_after_identity_verification(
        &organization,
        &VerifiedIdentity { issuer, subject },
        Duration::from_secs(lifetime_seconds),
    )
    .await
    .context("issue verified user session")?;
    eprintln!(
        "The user-session credential below is shown once. Keep it out of shell history and persistent browser storage."
    );
    print_json(&serde_json::json!({
        "session_id": session.session_id,
        "organization_id": session.organization_id,
        "user_id": session.user_id,
        "credential": session.credential,
        "expires_at": session.expires_at.format(&Rfc3339)?,
    }))
}

async fn verify(minimum_organizations: i64) -> Result<()> {
    if minimum_organizations < 0 {
        bail!("minimum organizations must be nonnegative");
    }
    let pool = database_pool().await?;
    let expected = [
        "organizations",
        "users",
        "organization_memberships",
        "projects",
        "environments",
        "data_sources",
        "behavior_schemas",
        "sdk_keys",
        "sampling_policies",
        "privacy_policies",
        "quotas",
        "feature_flags",
        "audit_log",
        "bootstrap_receipts",
        "user_identities",
        "user_sessions",
        "service_credentials",
        "collection_policies",
    ];
    let migration_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.chill_schema_migrations")
            .fetch_one(&pool)
            .await?;
    let table_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='control' AND table_name=ANY($1)")
        .bind(expected.as_slice()).fetch_one(&pool).await?;
    if usize::try_from(table_count).ok() != Some(expected.len()) {
        bail!(
            "control-plane table count = {table_count}, want {}",
            expected.len()
        );
    }
    let compliance_table_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='compliance' AND table_name='export_requests'")
        .fetch_one(&pool).await?;
    if compliance_table_count != 1 {
        bail!("compliance export request table is unavailable");
    }
    let unsafe_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns WHERE table_schema='control' AND table_name IN ('sdk_keys','service_credentials','user_sessions') AND column_name IN ('secret','raw_key','plaintext_key')")
        .fetch_one(&pool).await?;
    if unsafe_columns != 0 {
        bail!("credential table contains a plaintext-secret column");
    }
    let values: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(r"SELECT
        (SELECT count(*) FROM control.organizations),
        (SELECT count(*) FROM control.bootstrap_receipts),
        (SELECT count(*) FROM control.sdk_keys),
        (SELECT count(*) FROM control.sdk_keys WHERE octet_length(secret_digest)<>32),
        (SELECT count(*) FROM control.environments AS environment WHERE environment.status='active'
          AND NOT EXISTS (SELECT 1 FROM control.collection_policies AS policy
            WHERE policy.environment_id=environment.id AND policy.status='active')),
        (SELECT count(*) FROM compliance.export_requests WHERE status='completed'
          AND (octet_length(artifact_sha256)<>32 OR record_count<0 OR artifact_bytes<1 OR source_file_count<0))")
        .fetch_one(&pool).await?;
    if values.0 < minimum_organizations {
        bail!(
            "restored organization count = {}, want at least {minimum_organizations}",
            values.0
        );
    }
    if values.1 > values.2 || values.3 != 0 || values.4 != 0 || values.5 != 0 {
        bail!("restored SDK-key/bootstrap invariants are invalid");
    }
    print_json(&serde_json::json!({ "bootstrap_receipt_count": values.1,
        "compliance_table_count": compliance_table_count, "migration_count": migration_count,
        "organization_count": values.0, "sdk_key_count": values.2,
        "table_count": table_count, "verified": true }))
}

async fn verify_lake(maximum_files: usize) -> Result<()> {
    if !(1..=100_000).contains(&maximum_files) {
        bail!("maximum files must be between 1 and 100000");
    }
    let pool = database_pool().await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM lake.export_batches WHERE status='committed'")
            .fetch_one(&pool)
            .await?;
    if usize::try_from(count).unwrap_or(usize::MAX) > maximum_files {
        bail!("committed lake file count {count} exceeds verification limit {maximum_files}");
    }
    let objects = ImmutableStore::from_environment().await?;
    let rows: Vec<LakeLedgerRow> = sqlx::query_as(
        r"SELECT batch_id,object_key,manifest_key,object_sha256,manifest_sha256,byte_count,row_count
          FROM lake.export_batches WHERE status='committed'
          ORDER BY organization_id,project_id,environment_id,batch_id",
    )
    .fetch_all(&pool)
    .await?;
    let mut verified_rows = 0_i64;
    let mut verified_bytes = 0_i64;
    for (
        batch_id,
        object_key,
        manifest_key,
        object_digest,
        manifest_digest,
        byte_count,
        row_count,
    ) in rows
    {
        let object = objects.get(&object_key).await?;
        if i64::try_from(object.len()).ok() != Some(byte_count)
            || Sha256::digest(&object).as_slice() != object_digest.as_slice()
        {
            bail!("lake object {object_key} failed size or digest verification");
        }
        let manifest_body = objects.get(&manifest_key).await?;
        if Sha256::digest(&manifest_body).as_slice() != manifest_digest.as_slice() {
            bail!("lake manifest {manifest_key} failed digest verification");
        }
        let manifest: Manifest = serde_json::from_slice(&manifest_body)?;
        if manifest.batch_id != batch_id
            || manifest.object_key != object_key
            || i32::try_from(manifest.row_count).ok() != Some(row_count)
            || manifest.byte_count != byte_count
            || manifest.object_sha256 != hex::encode(&object_digest)
        {
            bail!("lake manifest {manifest_key} does not match its committed ledger row");
        }
        let decoded = decode_parquet(object)?;
        if i32::try_from(decoded.len()).ok() != Some(row_count)
            || decoded.iter().any(|row| {
                row.batch_id != batch_id || row.dataset_schema_version != DATASET_SCHEMA_VERSION
            })
        {
            bail!("lake object {object_key} contains invalid rows");
        }
        verified_rows = verified_rows.saturating_add(i64::from(row_count));
        verified_bytes = verified_bytes.saturating_add(byte_count);
    }
    print_json(
        &serde_json::json!({ "file_count": count, "row_count": verified_rows,
        "byte_count": verified_bytes, "dataset_schema_version": DATASET_SCHEMA_VERSION,
        "verified": true }),
    )
}

async fn execute_query(
    plan_path: PathBuf,
    organization_id: String,
    project_id: String,
    environment_id: String,
    cache_path: Option<PathBuf>,
) -> Result<()> {
    let plan: Plan = read_json(&plan_path, 1 << 20)?;
    let pool = database_pool().await?;
    let control = control_store(pool.clone())?;
    let objects = ImmutableStore::from_environment().await?;
    let cache_path = query_cache_path(cache_path);
    let files = FileCache::new(&cache_path, 2 << 30, objects)?;
    let engine = Arc::new(Engine::new(files.root(), EngineConfiguration::default())?);
    let service = QueryService::new(
        Catalog::new(control),
        files,
        ResultCache::new(16 << 20, 128, Duration::from_mins(1))?,
        engine,
        ServiceConfiguration::default(),
    )?;
    let result = service
        .execute(
            &Scope {
                organization_id,
                project_id,
                environment_id,
            },
            &plan,
        )
        .await?;
    print_json(&result)
}

#[allow(
    clippy::too_many_arguments,
    reason = "CLI flags map one-to-one to the audited export request"
)]
async fn privacy_export(
    kind: String,
    organization_id: String,
    project_id: String,
    environment_id: String,
    target_kind: Option<String>,
    target_file: Option<PathBuf>,
    requested_by: String,
    reason_code: String,
    idempotency_key: String,
    output: PathBuf,
) -> Result<()> {
    let pool = database_pool().await?;
    let exporter = Exporter::new(
        control_store(pool)?,
        ImmutableStore::from_environment().await?,
        ExportConfiguration::default(),
    )?;
    let request = ExportRequest {
        organization_id,
        project_id,
        environment_id,
        kind: parse_export_kind(&kind)?,
        target_kind: target_kind
            .as_deref()
            .map(parse_export_target)
            .transpose()?,
        target_value: read_private_target(target_file.as_deref())?,
        requested_by,
        reason_code,
        idempotency_key,
    };
    let receipt = exporter
        .run(request, |artifact| {
            write_private_artifact(&output, &artifact.body)
        })
        .await?;
    print_json(&receipt)
}

#[allow(
    clippy::too_many_arguments,
    reason = "CLI flags map one-to-one to the audited lifecycle request"
)]
async fn lifecycle_request(
    kind: String,
    organization_id: String,
    project_id: String,
    environment_id: String,
    target_kind: Option<String>,
    target_file: Option<PathBuf>,
    cutoff: Option<String>,
    requested_by: String,
    reason_code: String,
    idempotency_key: String,
    cache_path: Option<PathBuf>,
) -> Result<()> {
    let manager = lifecycle_manager(query_cache_path(cache_path)).await?;
    let cutoff_unix_nano = cutoff
        .map(|value| {
            let parsed = OffsetDateTime::parse(&value, &Rfc3339)
                .context("lifecycle cutoff must be an RFC3339 timestamp")?;
            u64::try_from(parsed.unix_timestamp_nanos())
                .context("lifecycle cutoff must be nonnegative")
        })
        .transpose()?;
    let created = manager
        .create_request(LifecycleRequest {
            id: String::new(),
            organization_id,
            project_id,
            environment_id,
            kind: parse_lifecycle_kind(&kind)?,
            target_kind: target_kind
                .as_deref()
                .map(parse_lifecycle_target)
                .transpose()?,
            target_value: read_private_target(target_file.as_deref())?,
            target_sha256: [0; 32],
            cutoff_unix_nano,
            requested_by,
            reason_code,
            idempotency_key,
            attempt_count: 0,
            created_at: None,
        })
        .await?;
    print_json(&created)
}

async fn lifecycle_run(
    maximum_requests: usize,
    schedule_retention: bool,
    cache_path: Option<PathBuf>,
) -> Result<()> {
    if !(1..=10_000).contains(&maximum_requests) {
        bail!("maximum requests must be between 1 and 10000");
    }
    let manager = lifecycle_manager(query_cache_path(cache_path)).await?;
    let scheduled = if schedule_retention {
        manager
            .schedule_due_retention(OffsetDateTime::now_utc())
            .await?
    } else {
        0
    };
    let mut processed = 0;
    while processed < maximum_requests {
        let count = manager.process_available().await?;
        if count == 0 {
            break;
        }
        processed += count;
    }
    print_json(&serde_json::json!({ "scheduled": scheduled, "processed": processed }))
}

async fn schema_propose(definition: PathBuf, compatibility: String) -> Result<()> {
    let schema: DatasetSchema = read_json(&definition, 1 << 20)?;
    let version = schema.version;
    let digest = SchemaRegistry::new(database_pool().await?)
        .propose(schema, parse_compatibility(&compatibility)?)
        .await?;
    print_json(&serde_json::json!({ "version": version, "status": "draft",
        "sha256": hex::encode(digest) }))
}

async fn lifecycle_manager(cache_path: PathBuf) -> Result<LifecycleManager> {
    let pool = database_pool().await?;
    let control = control_store(pool.clone())?;
    let objects = ImmutableStore::from_environment().await?;
    let mut configuration = LifecycleConfiguration::production("chillctl-lifecycle");
    configuration.query_cache_path = cache_path;
    Ok(LifecycleManager::new(
        pool,
        control,
        objects,
        configuration,
    )?)
}

fn control_store(pool: PgPool) -> Result<Store> {
    let pepper = decode_pepper(&required_environment("CHILL_KEY_PEPPER", 4096)?)?;
    Ok(Store::new(pool, CredentialIssuer::new(&pepper)?))
}

fn query_cache_path(explicit: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| env::var_os("CHILL_QUERY_CACHE_PATH").map(Into::into))
        .unwrap_or_else(|| PathBuf::from(".chill-data/query-cache"))
}

fn read_private_target(path: Option<&Path>) -> Result<String> {
    let Some(path) = path else {
        return Ok(String::new());
    };
    let body = fs::read(path).with_context(|| format!("read private target {}", path.display()))?;
    if body.len() > 257 {
        bail!("private target file exceeds 256 bytes");
    }
    let mut value = String::from_utf8(body).context("private target must be UTF-8")?;
    if value.ends_with('\n') {
        value.pop();
    }
    Ok(value)
}

fn write_private_artifact(path: &Path, body: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .context("privacy export output requires a parent directory")?;
    if !parent.is_dir() {
        bail!("privacy export output directory is unavailable");
    }
    let temporary = parent.join(format!(".chill-privacy-export-{}", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(body)?;
        file.sync_all()?;
        fs::hard_link(&temporary, path).context("publish privacy export artifact")?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn parse_export_kind(value: &str) -> Result<ExportKind> {
    match value {
        "tenant" => Ok(ExportKind::Tenant),
        "data_subject" => Ok(ExportKind::DataSubject),
        _ => bail!("export kind must be tenant or data_subject"),
    }
}

fn parse_export_target(value: &str) -> Result<ExportTargetKind> {
    match value {
        "installation_id" => Ok(ExportTargetKind::InstallationId),
        "session_id" => Ok(ExportTargetKind::SessionId),
        "replay_id" => Ok(ExportTargetKind::ReplayId),
        _ => bail!("export target kind is invalid"),
    }
}

fn parse_lifecycle_kind(value: &str) -> Result<LifecycleKind> {
    match value {
        "retention" => Ok(LifecycleKind::Retention),
        "replay_expiry" => Ok(LifecycleKind::ReplayExpiry),
        "data_subject" => Ok(LifecycleKind::DataSubject),
        "environment" => Ok(LifecycleKind::Environment),
        "tenant" => Ok(LifecycleKind::Tenant),
        _ => bail!("lifecycle kind is invalid"),
    }
}

fn parse_lifecycle_target(value: &str) -> Result<TargetKind> {
    match value {
        "installation_id" => Ok(TargetKind::InstallationId),
        "session_id" => Ok(TargetKind::SessionId),
        "replay_id" => Ok(TargetKind::ReplayId),
        _ => bail!("lifecycle target kind is invalid"),
    }
}

fn parse_compatibility(value: &str) -> Result<Compatibility> {
    match value {
        "backward" => Ok(Compatibility::Backward),
        "forward" => Ok(Compatibility::Forward),
        "full" => Ok(Compatibility::Full),
        _ => bail!("compatibility is invalid"),
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

async fn database_pool() -> Result<PgPool> {
    let database_url = required_environment("CHILL_DATABASE_URL", 4096)?;
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_mins(5))
        .max_lifetime(Duration::from_mins(10))
        .connect(&database_url)
        .await
        .context("connect to PostgreSQL")
}

async fn apply_migrations(pool: &PgPool) -> Result<Vec<chill_migrations::Migration>> {
    let runtime_role = env::var("CHILL_DATABASE_RUNTIME_ROLE")
        .ok()
        .filter(|value| !value.is_empty());
    chill_migrations::migrate_for_runtime_role(pool, runtime_role.as_deref())
        .await
        .context("apply migrations")
}

fn required_environment(name: &str, maximum_length: usize) -> Result<String> {
    let direct = env::var(name).ok();
    let file_name = format!("{name}_FILE");
    let file = env::var(&file_name).ok();
    if direct.is_some() && file.is_some() {
        bail!("{name} and {file_name} are mutually exclusive");
    }
    let value = if let Some(path) = file {
        if path.is_empty() {
            bail!("{file_name} is empty");
        }
        let metadata = fs::metadata(&path).with_context(|| format!("inspect {file_name}"))?;
        if !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > u64::try_from(maximum_length).unwrap_or(u64::MAX) + 1
        {
            bail!("{file_name} must be a regular file between 1 and {maximum_length} bytes");
        }
        let mut body = fs::read_to_string(path).with_context(|| format!("read {file_name}"))?;
        if body.ends_with('\n') {
            body.pop();
        }
        body
    } else {
        direct.with_context(|| format!("{name} is required"))?
    };
    if value.is_empty() || value.len() > maximum_length || value.contains('\0') {
        bail!("{name} must contain 1 to {maximum_length} non-NUL bytes");
    }
    Ok(value)
}

fn load_behavior_schema(explicit: Option<PathBuf>) -> Result<(serde_json::Value, PathBuf)> {
    let mut candidates = Vec::new();
    if let Some(path) =
        explicit.or_else(|| env::var("CHILL_BEHAVIOR_SCHEMA_PATH").ok().map(Into::into))
    {
        candidates.push(path);
    }
    candidates.extend([
        PathBuf::from("schemas/behavior/v1/envelope.schema.json"),
        PathBuf::from("../schemas/behavior/v1/envelope.schema.json"),
    ]);
    for path in candidates {
        match read_json::<serde_json::Value>(&path, 4 << 20) {
            Ok(value) if value.is_object() => return Ok((value, path)),
            Ok(_) => bail!("behavior schema {} must be a JSON object", path.display()),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read behavior schema {}", path.display()));
            }
        }
    }
    bail!("canonical behavior schema was not found; pass --schema")
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf, maximum_bytes: usize) -> Result<T> {
    let body = fs::read(path)?;
    if body.len() > maximum_bytes {
        bail!("{} exceeds {maximum_bytes} bytes", path.display());
    }
    serde_json::from_slice(&body).context("decode strict JSON")
}

fn decode_pepper(encoded: &str) -> Result<Vec<u8>> {
    let encoded = encoded.trim();
    if encoded.is_empty() || encoded.len() > 4096 {
        bail!("CHILL_KEY_PEPPER must contain 1 to 4096 bytes");
    }
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE_NO_PAD] {
        if let Ok(value) = engine.decode(encoded) {
            return Ok(value);
        }
    }
    hex::decode(encoded).context("CHILL_KEY_PEPPER must be base64 or hex")
}

fn local_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_else(|error| unreachable!("static HTTP client configuration failed: {error}"))
}
