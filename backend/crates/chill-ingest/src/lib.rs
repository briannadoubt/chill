//! Authenticated, bounded, idempotent, durable ingestion boundaries.

mod grpc;
mod http;
mod service;
mod types;
mod validate;

pub use grpc::grpc_router;
pub use http::ingest_router;
pub use service::Service;
pub use types::{
    AdmissionError, Candidate, ErrorCode, Limits, PayloadFormat, Receipt, ReplayMetadata,
    SchemaRef, SignalKind, ValidatedRequest,
};
pub use validate::validate_candidate;
