//! Rust process assembly and operational endpoints for the Chill server.

mod alerts;
mod metrics;

use std::{env, fs, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{
        HeaderValue, Method, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, HeaderName},
    },
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chill_control_plane::{
    CredentialIssuer, SitesAuthentication, Store, collection_router, console_router,
    sites_auth_router,
};
use chill_ingest::{Limits as IngestLimits, Service as IngestService, grpc_router, ingest_router};
use chill_lake::{Processor as LakeProcessor, ProcessorConfiguration as LakeConfiguration};
use chill_lifecycle::{Configuration as LifecycleConfiguration, Manager as LifecycleManager};
use chill_normalize::{Decoder, Processor, ProcessorConfiguration, TimingPolicy};
use chill_objects::ImmutableStore;
use chill_query::{
    Catalog as QueryCatalog, Engine as QueryEngine,
    EngineConfiguration as QueryEngineConfiguration, FileCache as QueryFileCache,
    ResultCache as QueryResultCache, Service as QueryService,
    ServiceConfiguration as QueryConfiguration, query_router,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::metrics::Metrics;

const CONFIGURED_VERSION: Option<&str> = option_env!("CHILL_VERSION");
const COMMIT: &str = match option_env!("CHILL_COMMIT") {
    Some(value) => value,
    None => "unknown",
};
const BUILD_TIME: &str = match option_env!("CHILL_BUILD_TIME") {
    Some(value) => value,
    None => "unknown",
};

#[derive(Clone)]
struct ApplicationState {
    database: PgPool,
    metrics: Arc<Metrics>,
}

#[tokio::main]
async fn main() -> Result<()> {
    initialize_logging();
    let database_url = required_environment("CHILL_DATABASE_URL", 4096)?;
    let database = PgPoolOptions::new()
        .max_connections(16)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_mins(5))
        .max_lifetime(Duration::from_mins(10))
        .connect(&database_url)
        .await
        .context("connect to PostgreSQL")?;
    let pepper = decode_pepper(&required_environment("CHILL_KEY_PEPPER", 4096)?)?;
    let control_plane = Store::new(
        database.clone(),
        CredentialIssuer::new(&pepper).context("validate CHILL_KEY_PEPPER")?,
    );
    let address = env::var("CHILL_LISTEN_ADDRESS")
        .unwrap_or_else(|_| "0.0.0.0:4318".to_owned())
        .parse::<SocketAddr>()
        .context("parse CHILL_LISTEN_ADDRESS")?;
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("bind {address}"))?;
    let metrics = Arc::new(Metrics::default());
    let operational = operational_router(database.clone(), Arc::clone(&metrics));
    let ingest_limits = IngestLimits::default();
    let ingestion = IngestService::new(control_plane.clone(), ingest_limits)
        .context("configure ingestion service")?;
    let normalizer = Processor::new(
        database.clone(),
        control_plane.clone(),
        Decoder::new(TimingPolicy::default()),
        ProcessorConfiguration::production(format!("chilld-{}", std::process::id())),
    )
    .context("configure normalization worker")?;
    let normalizer_task = tokio::spawn(async move { normalizer.run().await });
    let objects = ImmutableStore::from_environment()
        .await
        .context("configure object store")?;
    let query_service = configure_query(control_plane.clone(), objects.clone())?;
    let alert_task = start_alerts(database.clone(), Arc::clone(&query_service))?;
    let lake = LakeProcessor::new(
        database.clone(),
        control_plane.clone(),
        objects.clone(),
        LakeConfiguration::production(format!("chilld-lake-{}", std::process::id())),
    )
    .context("configure lake publisher")?;
    let lake_task = tokio::spawn(async move { lake.run().await });
    let lifecycle_tasks = if optional_boolean("CHILL_LIFECYCLE_ENABLED", true)? {
        let mut lifecycle_configuration =
            LifecycleConfiguration::production(format!("chilld-lifecycle-{}", std::process::id()));
        lifecycle_configuration.query_cache_path = env::var_os("CHILL_QUERY_CACHE_PATH")
            .map_or_else(|| ".chill-data/query-cache".into(), Into::into);
        let lifecycle = LifecycleManager::new(
            database,
            control_plane.clone(),
            objects,
            lifecycle_configuration,
        )
        .context("configure lifecycle worker")?;
        let worker = lifecycle.clone();
        let worker_task = tokio::spawn(async move { worker.run().await });
        let scheduler_task = tokio::spawn(async move { schedule_retention(lifecycle).await });
        vec![worker_task, scheduler_task]
    } else {
        Vec::new()
    };
    let mut application = operational
        .merge(console_router(control_plane.clone()))
        .merge(collection_router(control_plane.clone()))
        .merge(query_router(control_plane.clone(), query_service))
        .merge(ingest_router(ingestion.clone(), ingest_limits))
        .merge(grpc_router(ingestion))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&metrics),
            observe_http,
        ))
        .layer(middleware::from_fn(add_version_header));
    if let Some(authentication) = sites_authentication()? {
        application = application.merge(sites_auth_router(control_plane, authentication));
    }
    let application = with_console_cors(application)?;

    info!(%address, "chilld listening");
    let result = axum::serve(listener, application)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve HTTP");
    normalizer_task.abort();
    lake_task.abort();
    if let Some(task) = alert_task {
        task.abort();
    }
    for task in lifecycle_tasks {
        task.abort();
    }
    result
}

fn operational_router(database: PgPool, metrics: Arc<Metrics>) -> Router {
    Router::new()
        .route("/healthz", get(livez))
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .route("/metrics", get(prometheus_metrics))
        .route("/version", get(version))
        .with_state(ApplicationState { database, metrics })
}

fn configure_query(control: Store, objects: ImmutableStore) -> Result<Arc<QueryService>> {
    let cache = env::var_os("CHILL_QUERY_CACHE_PATH").map_or_else(
        || ".chill-data/query-cache".into(),
        std::path::PathBuf::from,
    );
    let files = QueryFileCache::new(cache.join("files"), 8_i64 << 30, objects)
        .context("configure query file cache")?;
    let engine = Arc::new(
        QueryEngine::new(files.root(), QueryEngineConfiguration::default())
            .context("configure query engine")?,
    );
    Ok(Arc::new(
        QueryService::new(
            QueryCatalog::new(control),
            files,
            QueryResultCache::new(64 << 20, 256, Duration::from_mins(1))
                .context("configure query result cache")?,
            engine,
            QueryConfiguration::default(),
        )
        .context("configure query service")?,
    ))
}

fn start_alerts(
    database: PgPool,
    query: Arc<QueryService>,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    Ok(optional_boolean("CHILL_ALERTS_ENABLED", true)?.then(|| {
        tokio::spawn(async move {
            alerts::run(database, query).await;
        })
    }))
}

fn optional_console_origin() -> Result<Option<HeaderValue>> {
    env::var("CHILL_CONSOLE_ORIGIN")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<HeaderValue>()
                .context("parse CHILL_CONSOLE_ORIGIN")
        })
        .transpose()
}

fn with_console_cors(application: Router) -> Result<Router> {
    Ok(if let Some(origin) = optional_console_origin()? {
        application.layer(
            CorsLayer::new()
                .allow_origin(origin)
                .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE]),
        )
    } else {
        application
    })
}

fn sites_authentication() -> Result<Option<SitesAuthentication>> {
    let configured = env::var_os("CHILL_SITES_AUTH_SECRET").is_some()
        || env::var_os("CHILL_SITES_AUTH_SECRET_FILE").is_some();
    configured
        .then(|| {
            SitesAuthentication::new(&required_environment("CHILL_SITES_AUTH_SECRET", 512)?)
                .context("validate CHILL_SITES_AUTH_SECRET")
        })
        .transpose()
}

async fn schedule_retention(manager: LifecycleManager) {
    let mut interval = tokio::time::interval(Duration::from_hours(6));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match manager
            .schedule_due_retention(OffsetDateTime::now_utc())
            .await
        {
            Ok(created) => info!(created, "retention scheduling pass completed"),
            Err(cause) => error!(%cause, "retention scheduling pass failed"),
        }
    }
}

async fn livez() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn readyz(State(state): State<ApplicationState>) -> StatusCode {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.database)
        .await
    {
        Ok(1) => StatusCode::NO_CONTENT,
        Ok(value) => {
            error!(value, "unexpected PostgreSQL readiness result");
            StatusCode::SERVICE_UNAVAILABLE
        }
        Err(error) => {
            error!(%error, "PostgreSQL readiness check failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

async fn prometheus_metrics(State(state): State<ApplicationState>) -> String {
    state
        .metrics
        .render(state.database.size(), state.database.num_idle())
}

async fn observe_http(
    State(metrics): State<Arc<Metrics>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    metrics.started();
    let started = std::time::Instant::now();
    let response = next.run(request).await;
    metrics.finished(&path, response.status().as_u16(), started.elapsed());
    response
}

async fn version() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({ "version": version_value(), "commit": COMMIT, "build_time": BUILD_TIME }),
    )
}

async fn add_version_header(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(version_value()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-chill-version"), value);
    }
    response
}

fn version_value() -> &'static str {
    match CONFIGURED_VERSION {
        Some(value) if !value.is_empty() && !matches!(value, "dev" | "unknown") => value,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        error!(%error, "failed to install shutdown signal handler");
    }
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
        let mut value = fs::read_to_string(&path).with_context(|| format!("read {file_name}"))?;
        if value.ends_with('\n') {
            value.pop();
        }
        value
    } else {
        direct.with_context(|| format!("{name} is required"))?
    };
    if value.is_empty() || value.len() > maximum_length || value.contains('\0') {
        bail!("{name} must contain 1 to {maximum_length} non-NUL bytes");
    }
    Ok(value)
}

fn optional_boolean(name: &str, fallback: bool) -> Result<bool> {
    match env::var(name) {
        Ok(value) if !value.is_empty() => value
            .parse::<bool>()
            .with_context(|| format!("{name} must be a boolean")),
        Ok(_) | Err(env::VarError::NotPresent) => Ok(fallback),
        Err(error) => Err(error).with_context(|| format!("read {name}")),
    }
}

fn initialize_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();
}

fn decode_pepper(encoded: &str) -> Result<Vec<u8>> {
    let encoded = encoded.trim();
    if encoded.is_empty() {
        bail!("CHILL_KEY_PEPPER is empty");
    }
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE_NO_PAD] {
        if let Ok(value) = engine.decode(encoded) {
            return Ok(value);
        }
    }
    hex::decode(encoded).context("CHILL_KEY_PEPPER must be base64 or hex")
}
