//! Bounded, typed analytical query plans and safe `DuckDB` compilation.

mod catalog;
mod compiler;
mod engine;
mod file_cache;
mod http;
mod model;
mod result_cache;
mod service;

pub use catalog::{Catalog, CatalogError, Dataset, LakeFile};
pub use compiler::{Argument, Compiled, compile};
pub use engine::{Engine, EngineConfiguration, EngineError};
pub use file_cache::{AcquiredFiles, FileCache, FileCacheError};
pub use http::query_router;
pub use model::{
    AggregatePlan, BehaviorFilter, CohortPlan, Cursor, EventsPlan, FunnelPlan, FunnelStep, Kind,
    PLAN_VERSION, PathPlan, Plan, QueryError, QueryResult as Result, ReplayPlan, RetentionPlan,
    Scope, Stats, TimeRange, TracePlan,
};
pub use result_cache::{ResultCache, ResultCacheError};
pub use service::{Configuration as ServiceConfiguration, Service, ServiceError};
