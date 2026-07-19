use std::time::Duration;

use axum::Router;
use bytes::Bytes;
use opentelemetry_proto::tonic::collector::{
    logs::v1::{
        ExportLogsServiceRequest, ExportLogsServiceResponse,
        logs_service_server::{LogsService, LogsServiceServer},
    },
    metrics::v1::{
        ExportMetricsServiceRequest, ExportMetricsServiceResponse,
        metrics_service_server::{MetricsService, MetricsServiceServer},
    },
    trace::v1::{
        ExportTraceServiceRequest, ExportTraceServiceResponse,
        trace_service_server::{TraceService, TraceServiceServer},
    },
};
use prost::Message;
use tonic::{Code, Request, Response, Status, metadata::MetadataMap, service::Routes};

use crate::{AdmissionError, Candidate, ErrorCode, PayloadFormat, Receipt, Service, SignalKind};

#[derive(Clone)]
struct Adapter {
    service: Service,
}

/// Builds OTLP/gRPC services suitable for merging into the shared h2c Axum listener.
pub fn grpc_router(service: Service) -> Router {
    let adapter = Adapter { service };
    Routes::new(LogsServiceServer::new(adapter.clone()))
        .add_service(TraceServiceServer::new(adapter.clone()))
        .add_service(MetricsServiceServer::new(adapter))
        .into_axum_router()
}

#[tonic::async_trait]
impl LogsService for Adapter {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        self.accept(request, SignalKind::Logs)
            .await
            .map(|receipt| response(ExportLogsServiceResponse::default(), &receipt))
    }
}

#[tonic::async_trait]
impl TraceService for Adapter {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        self.accept(request, SignalKind::Traces)
            .await
            .map(|receipt| response(ExportTraceServiceResponse::default(), &receipt))
    }
}

#[tonic::async_trait]
impl MetricsService for Adapter {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        self.accept(request, SignalKind::Metrics)
            .await
            .map(|receipt| response(ExportMetricsServiceResponse::default(), &receipt))
    }
}

impl Adapter {
    async fn accept<T>(&self, request: Request<T>, kind: SignalKind) -> Result<Receipt, Status>
    where
        T: Message,
    {
        let credential = grpc_credential(request.metadata())?;
        let idempotency = metadata_values(request.metadata(), "idempotency-key")?;
        if idempotency.len() > 1 {
            return Err(Status::invalid_argument(
                "multiple idempotency keys are not allowed",
            ));
        }
        self.service
            .accept(Candidate {
                kind,
                format: PayloadFormat::Protobuf,
                payload: request.into_inner().encode_to_vec(),
                credential,
                idempotency_key: idempotency.into_iter().next().unwrap_or_default(),
                replay: None,
            })
            .await
            .map_err(grpc_error)
    }
}

fn grpc_credential(metadata: &MetadataMap) -> Result<String, Status> {
    let mut values = Vec::new();
    for value in metadata_values(metadata, "authorization")? {
        let credential = value
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Status::unauthenticated("SDK key is invalid"))?;
        values.push(credential.to_owned());
    }
    values.extend(metadata_values(metadata, "x-chill-sdk-key")?);
    if values.len() == 1 && !values[0].is_empty() {
        values
            .pop()
            .ok_or_else(|| Status::unauthenticated("SDK key is invalid"))
    } else {
        Err(Status::unauthenticated("SDK key is invalid"))
    }
}

fn metadata_values(metadata: &MetadataMap, name: &'static str) -> Result<Vec<String>, Status> {
    metadata
        .get_all(name)
        .iter()
        .map(|value| {
            value
                .to_str()
                .map(str::to_owned)
                .map_err(|_| Status::invalid_argument("metadata is invalid"))
        })
        .collect()
}

fn response<T>(body: T, receipt: &Receipt) -> Response<T> {
    let mut response = Response::new(body);
    for (name, value) in [
        ("idempotency-key", receipt.idempotency_key.clone()),
        ("chill-inbox-id", receipt.inbox_id.to_string()),
        ("chill-duplicate", receipt.duplicate.to_string()),
    ] {
        if let Ok(value) = value.parse() {
            response.metadata_mut().insert(name, value);
        }
    }
    response
}

fn grpc_error(error: AdmissionError) -> Status {
    let code = match error.code {
        ErrorCode::Invalid => Code::InvalidArgument,
        ErrorCode::Unauthorized => Code::Unauthenticated,
        ErrorCode::Forbidden => Code::PermissionDenied,
        ErrorCode::TooLarge | ErrorCode::Quota | ErrorCode::Backpressure => Code::ResourceExhausted,
        ErrorCode::Conflict => Code::FailedPrecondition,
        ErrorCode::Unavailable => Code::Unavailable,
    };
    if let Some(delay) = error.retry_after {
        let detail = RetryInfo {
            retry_delay: Some(duration_proto(delay)),
        };
        let status = GoogleStatus {
            code: code as i32,
            message: error.message.clone(),
            details: vec![prost_types::Any {
                type_url: "type.googleapis.com/google.rpc.RetryInfo".to_owned(),
                value: detail.encode_to_vec(),
            }],
        };
        return Status::with_details(code, error.message, Bytes::from(status.encode_to_vec()));
    }
    Status::new(code, error.message)
}

fn duration_proto(duration: Duration) -> prost_types::Duration {
    let bounded = duration.clamp(Duration::from_secs(1), Duration::from_hours(1));
    prost_types::Duration {
        seconds: i64::try_from(bounded.as_secs()).unwrap_or(3600),
        nanos: i32::try_from(bounded.subsec_nanos()).unwrap_or_default(),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct GoogleStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<prost_types::Any>,
}

#[derive(Clone, Copy, PartialEq, prost::Message)]
struct RetryInfo {
    #[prost(message, optional, tag = "1")]
    retry_delay: Option<prost_types::Duration>,
}
