use std::{sync::Arc, time::Duration};

use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio::{sync::Semaphore, task::JoinError, time::Instant};

use crate::{
    Catalog, CatalogError, Engine, EngineError, FileCache, FileCacheError, Plan, QueryError,
    Result as QueryResult, ResultCache, ResultCacheError, Scope, compile,
};

/// End-to-end query resource bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Configuration {
    /// Maximum requested time range.
    pub maximum_range: Duration,
    /// Maximum immutable input files.
    pub maximum_files: usize,
    /// Maximum declared compressed scan bytes.
    pub maximum_scan_bytes: i64,
    /// Maximum returned rows.
    pub maximum_result_rows: usize,
    /// Maximum serialized row bytes.
    pub maximum_result_bytes: usize,
    /// `DuckDB` execution deadline.
    pub query_timeout: Duration,
    /// Queue wait deadline.
    pub queue_timeout: Duration,
    /// Concurrent embedded-engine queries. Must remain one.
    pub maximum_concurrent: usize,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            maximum_range: Duration::from_hours(744),
            maximum_files: 1_000,
            maximum_scan_bytes: 1 << 30,
            maximum_result_rows: 5_000,
            maximum_result_bytes: 8 << 20,
            query_timeout: Duration::from_secs(10),
            queue_timeout: Duration::from_millis(200),
            maximum_concurrent: 1,
        }
    }
}

/// Query orchestration failure.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// Dependencies or resource bounds are invalid.
    #[error("query service configuration is invalid")]
    InvalidConfiguration,
    /// Typed request validation failed.
    #[error("invalid query request: {0}")]
    Invalid(#[from] QueryError),
    /// Tenant catalog resolution failed.
    #[error("query catalog failed: {0}")]
    Catalog(#[from] CatalogError),
    /// Immutable file materialization failed.
    #[error("query file materialization failed: {0}")]
    Files(#[from] FileCacheError),
    /// Result cache operation failed.
    #[error("query result cache failed: {0}")]
    Results(#[from] ResultCacheError),
    /// Embedded execution failed.
    #[error("query execution failed: {0}")]
    Engine(#[from] EngineError),
    /// Blocking engine task failed.
    #[error("query execution task failed: {0}")]
    Join(#[from] JoinError),
    /// Queue could not be entered before its deadline.
    #[error("query service is busy")]
    Busy,
    /// Query exceeded a file, scan, result, or cache bound.
    #[error("query exceeds a resource limit")]
    ResourceLimit,
    /// Execution exceeded its deadline and was interrupted.
    #[error("query execution deadline exceeded")]
    Deadline,
    /// Result cache key serialization failed.
    #[error("query cache key serialization failed")]
    CacheKey,
}

/// Tenant-safe, cached, bounded query service.
pub struct Service {
    catalog: Catalog,
    files: FileCache,
    results: ResultCache,
    engine: Arc<Engine>,
    configuration: Configuration,
    semaphore: Arc<Semaphore>,
}

impl Service {
    /// Creates a service around one locked embedded engine.
    ///
    /// # Errors
    ///
    /// Returns an error when any resource bound is unsafe or the concurrency
    /// contract is not exactly one.
    pub fn new(
        catalog: Catalog,
        files: FileCache,
        results: ResultCache,
        engine: Arc<Engine>,
        configuration: Configuration,
    ) -> std::result::Result<Self, ServiceError> {
        if configuration.maximum_range.is_zero()
            || configuration.maximum_files == 0
            || configuration.maximum_scan_bytes < 1 << 20
            || configuration.maximum_result_rows == 0
            || configuration.maximum_result_bytes < 1_024
            || configuration.query_timeout.is_zero()
            || configuration.queue_timeout.is_zero()
            || configuration.maximum_concurrent != 1
        {
            return Err(ServiceError::InvalidConfiguration);
        }
        Ok(Self {
            catalog,
            files,
            results,
            engine,
            configuration,
            semaphore: Arc::new(Semaphore::new(configuration.maximum_concurrent)),
        })
    }

    /// Resolves, materializes, compiles, executes, and caches one typed query.
    ///
    /// # Errors
    ///
    /// Returns a classified error for validation, busy/deadline, catalog,
    /// resource, cache, materialization, or engine failures.
    #[allow(
        clippy::too_many_lines,
        reason = "query execution owns one resource lease across planning, cache pins, and blocking engine work"
    )]
    pub async fn execute(
        &self,
        scope: &Scope,
        plan: &Plan,
    ) -> std::result::Result<QueryResult, ServiceError> {
        let started = Instant::now();
        scope.validate()?;
        plan.validate(
            self.configuration.maximum_range,
            self.configuration.maximum_result_rows,
        )?;
        let queue_started = Instant::now();
        let permit = tokio::time::timeout(
            self.configuration.queue_timeout,
            Arc::clone(&self.semaphore).acquire_owned(),
        )
        .await
        .map_err(|_| ServiceError::Busy)?
        .map_err(|_| ServiceError::Busy)?;
        let queue_duration = queue_started.elapsed();
        let planning_started = Instant::now();
        let dataset = self
            .catalog
            .resolve(
                scope,
                plan,
                self.configuration.maximum_files,
                self.configuration.maximum_scan_bytes,
            )
            .await
            .map_err(|error| {
                if matches!(error, CatalogError::ResourceLimit) {
                    ServiceError::ResourceLimit
                } else {
                    ServiceError::Catalog(error)
                }
            })?;
        let planning_duration = planning_started.elapsed();
        if dataset.files.len() > self.configuration.maximum_files
            || dataset.byte_count > self.configuration.maximum_scan_bytes
        {
            return Err(ServiceError::ResourceLimit);
        }
        let key = result_cache_key(scope, plan, &dataset.generation)?;
        if let Some(mut cached) = self.results.get(&key)? {
            cached.stats.queue_duration_nano = nanos(queue_duration);
            cached.stats.planning_duration_nano = nanos(planning_duration);
            cached.stats.materialize_duration_nano = 0;
            cached.stats.execution_duration_nano = 0;
            cached.stats.total_duration_nano = nanos(started.elapsed());
            drop(permit);
            return Ok(cached);
        }
        let materialize_started = Instant::now();
        let acquired = self.files.acquire(&dataset.files).await.map_err(|error| {
            if matches!(error, FileCacheError::Capacity) {
                ServiceError::ResourceLimit
            } else {
                ServiceError::Files(error)
            }
        })?;
        let materialize_duration = materialize_started.elapsed();
        let compiled = match compile(plan, &acquired.paths) {
            Ok(compiled) => compiled,
            Err(error) => {
                acquired.release().await;
                return Err(ServiceError::Invalid(error));
            }
        };
        let engine = Arc::clone(&self.engine);
        let maximum_rows = self.configuration.maximum_result_rows;
        let maximum_bytes = self.configuration.maximum_result_bytes;
        let execution_started = Instant::now();
        let mut task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            engine.execute(&compiled, maximum_rows, maximum_bytes)
        });
        let execution = tokio::time::timeout(self.configuration.query_timeout, &mut task).await;
        let mut result = if let Ok(joined) = execution {
            match joined {
                Err(error) => {
                    acquired.release().await;
                    return Err(ServiceError::Join(error));
                }
                Ok(Ok(result)) => result,
                Ok(Err(EngineError::ResultLimit)) => {
                    acquired.release().await;
                    return Err(ServiceError::ResourceLimit);
                }
                Ok(Err(error)) => {
                    acquired.release().await;
                    return Err(ServiceError::Engine(error));
                }
            }
        } else {
            self.engine.interrupt();
            let _ = task.await;
            acquired.release().await;
            return Err(ServiceError::Deadline);
        };
        let execution_duration = execution_started.elapsed();
        acquired.release().await;
        result.stats.manifest_generation = dataset.generation;
        result.stats.file_count = dataset.files.len();
        result.stats.scan_bytes = dataset.byte_count;
        result.stats.row_count = result.rows.len();
        result.stats.queue_duration_nano = nanos(queue_duration);
        result.stats.planning_duration_nano = nanos(planning_duration);
        result.stats.materialize_duration_nano = nanos(materialize_duration);
        result.stats.execution_duration_nano = nanos(execution_duration);
        result.stats.total_duration_nano = nanos(started.elapsed());
        self.results.put(key, &result)?;
        Ok(result)
    }
}

fn result_cache_key(
    scope: &Scope,
    plan: &Plan,
    generation: &str,
) -> std::result::Result<String, ServiceError> {
    let body =
        serde_json::to_vec(&(scope, plan, generation)).map_err(|_| ServiceError::CacheKey)?;
    Ok(hex::encode(Sha256::digest(body)))
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}
