use std::time::Duration;

use axum::{
    Router,
    body::Body,
    extract::Request as AxumRequest,
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response as AxumResponse},
};
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

use crate::{
    AdmissionError, Candidate, ErrorCode, PayloadFormat, Receipt, Service, SignalKind,
    service::AdmissionPermit,
};

#[derive(Clone)]
struct Adapter {
    service: Service,
}

/// Builds OTLP/gRPC services suitable for merging into the shared h2c Axum listener.
pub fn grpc_router(service: Service) -> Router {
    let adapter = Adapter {
        service: service.clone(),
    };
    let limits = service.limits();
    Routes::new(
        LogsServiceServer::new(adapter.clone())
            .max_decoding_message_size(limits.maximum_payload_bytes),
    )
    .add_service(
        TraceServiceServer::new(adapter.clone())
            .max_decoding_message_size(limits.maximum_payload_bytes),
    )
    .add_service(
        MetricsServiceServer::new(adapter).max_decoding_message_size(limits.maximum_payload_bytes),
    )
    .into_axum_router()
    .layer(middleware::from_fn_with_state(
        service,
        grpc_admission_middleware,
    ))
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
        T: BoundedExportRequest,
    {
        let mut request = request;
        let credential = grpc_credential(request.metadata())?;
        let idempotency = metadata_values(request.metadata(), "idempotency-key")?;
        if idempotency.len() > 1 {
            return Err(Status::invalid_argument(
                "multiple idempotency keys are not allowed",
            ));
        }
        let permit = request
            .extensions_mut()
            .remove::<AdmissionPermit>()
            .map_or_else(|| self.service.try_admit().map_err(grpc_error), Ok)?;
        let message = request.into_inner();
        enforce_resource_container_limit(&message, self.service.limits().maximum_records)
            .map_err(grpc_error)?;
        let payload = bounded_encode(&message, self.service.limits().maximum_payload_bytes)
            .map_err(grpc_error)?;
        self.service
            .accept_admitted(
                Candidate {
                    kind,
                    format: PayloadFormat::Protobuf,
                    payload,
                    credential,
                    idempotency_key: idempotency.into_iter().next().unwrap_or_default(),
                    replay: None,
                },
                permit,
            )
            .await
            .map_err(grpc_error)
    }
}

trait BoundedExportRequest: Message {
    fn resource_container_count(&self) -> usize;

    fn resource_container_name() -> &'static str;
}

impl BoundedExportRequest for ExportLogsServiceRequest {
    fn resource_container_count(&self) -> usize {
        self.resource_logs.len()
    }

    fn resource_container_name() -> &'static str {
        "resource_logs"
    }
}

impl BoundedExportRequest for ExportTraceServiceRequest {
    fn resource_container_count(&self) -> usize {
        self.resource_spans.len()
    }

    fn resource_container_name() -> &'static str {
        "resource_spans"
    }
}

impl BoundedExportRequest for ExportMetricsServiceRequest {
    fn resource_container_count(&self) -> usize {
        self.resource_metrics.len()
    }

    fn resource_container_name() -> &'static str {
        "resource_metrics"
    }
}

async fn grpc_admission_middleware(
    service: axum::extract::State<Service>,
    mut request: AxumRequest<Body>,
    next: Next,
) -> AxumResponse {
    let permit = match service.try_admit() {
        Ok(value) => value,
        Err(error) => return grpc_admission_error(&error),
    };
    request.extensions_mut().insert(permit);
    next.run(request).await
}

fn enforce_resource_container_limit<T: BoundedExportRequest>(
    request: &T,
    maximum_records: usize,
) -> Result<(), AdmissionError> {
    if request.resource_container_count() > maximum_records {
        return Err(AdmissionError::new(
            ErrorCode::TooLarge,
            format!(
                "gRPC {} count exceeds the configured record limit",
                T::resource_container_name()
            ),
        ));
    }
    Ok(())
}

fn bounded_encode<T: Message>(
    message: &T,
    maximum_payload_bytes: usize,
) -> Result<Vec<u8>, AdmissionError> {
    let encoded_len = message.encoded_len();
    if encoded_len > maximum_payload_bytes {
        return Err(AdmissionError::new(
            ErrorCode::TooLarge,
            "request payload exceeds the configured limit",
        ));
    }
    let mut payload = Vec::with_capacity(encoded_len);
    message.encode(&mut payload).map_err(|error| {
        AdmissionError::with_source(ErrorCode::Invalid, "request payload is invalid", error)
    })?;
    Ok(payload)
}

fn grpc_admission_error(error: &AdmissionError) -> AxumResponse {
    let mut response = (StatusCode::OK, Body::empty()).into_response();
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("application/grpc"));
    response
        .headers_mut()
        .insert("grpc-status", HeaderValue::from_static("8"));
    response.headers_mut().insert(
        "grpc-message",
        HeaderValue::from_static("ingestion%20concurrency%20is%20saturated"),
    );
    if let Some(retry_after) = error.retry_after
        && let Ok(value) = HeaderValue::try_from(retry_after.as_secs().max(1).to_string())
    {
        response.headers_mut().insert("retry-after", value);
    }
    response
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use opentelemetry_proto::tonic::logs::v1::ResourceLogs;

    #[test]
    fn rejects_too_many_grpc_resource_containers_before_reencoding() -> anyhow::Result<()> {
        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs::default(), ResourceLogs::default()],
        };

        let Err(error) = enforce_resource_container_limit(&request, 1) else {
            bail!("excessive resource_logs should be rejected");
        };

        assert_eq!(error.code, ErrorCode::TooLarge);
        assert!(error.message.contains("resource_logs"));
        Ok(())
    }

    #[test]
    fn rejects_reencoded_grpc_payloads_over_configured_limit() -> anyhow::Result<()> {
        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs::default()],
        };

        let Err(error) = bounded_encode(&request, 1) else {
            bail!("oversized encoded payload should be rejected");
        };

        assert_eq!(error.code, ErrorCode::TooLarge);
        Ok(())
    }
}
