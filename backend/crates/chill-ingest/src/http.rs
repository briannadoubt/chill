use std::io::Read as _;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use flate2::read::GzDecoder;
use prost::Message as _;

use crate::{
    AdmissionError, Candidate, ErrorCode, Limits, PayloadFormat, Receipt, ReplayMetadata, Service,
    SignalKind,
};

const REPLAY_MEDIA_TYPE: &str = "application/vnd.chill.replay.v1+octet-stream";

#[derive(Clone)]
struct HttpState {
    service: Service,
    limits: Limits,
}

/// Builds the bounded OTLP/HTTP intake routes.
pub fn ingest_router(service: Service, limits: Limits) -> Router {
    Router::new()
        .route("/v1/logs", post(logs))
        .route("/v1/traces", post(traces))
        .route("/v1/metrics", post(metrics))
        .route("/v1/chill/replay", post(replay))
        .layer(middleware::from_fn(security_headers))
        .with_state(HttpState { service, limits })
}

async fn logs(State(state): State<HttpState>, request: Request) -> Response {
    otlp(state, request, SignalKind::Logs).await
}

async fn traces(State(state): State<HttpState>, request: Request) -> Response {
    otlp(state, request, SignalKind::Traces).await
}

async fn metrics(State(state): State<HttpState>, request: Request) -> Response {
    otlp(state, request, SignalKind::Metrics).await
}

async fn replay(State(state): State<HttpState>, request: Request) -> Response {
    if media_type_header(request.headers()) != REPLAY_MEDIA_TYPE {
        return error_response(
            PayloadFormat::Json,
            AdmissionError::new(ErrorCode::Invalid, "unsupported replay content type"),
        );
    }
    let (parts, body) = request.into_parts();
    let credential = match sdk_credential(&parts.headers) {
        Ok(value) => value,
        Err(error) => return error_response(PayloadFormat::Json, error),
    };
    let metadata = match replay_metadata(&parts.headers) {
        Ok(value) => value,
        Err(error) => return error_response(PayloadFormat::Json, error),
    };
    let idempotency_key = header_text(&parts.headers, "idempotency-key").unwrap_or_default();
    let payload = match bounded_body(body, &parts.headers, state.limits.maximum_replay_bytes).await
    {
        Ok(value) => value,
        Err(error) => return error_response(PayloadFormat::Json, error),
    };
    match state
        .service
        .accept(Candidate {
            kind: SignalKind::Replay,
            format: PayloadFormat::ReplayV1,
            payload,
            credential,
            idempotency_key,
            replay: Some(metadata),
        })
        .await
    {
        Ok(receipt) => replay_success(&receipt),
        Err(error) => error_response(PayloadFormat::Json, error),
    }
}

async fn otlp(state: HttpState, request: Request, kind: SignalKind) -> Response {
    let format = match otlp_format(request.headers()) {
        Ok(value) => value,
        Err(error) => return error_response(PayloadFormat::Json, error),
    };
    let (parts, body) = request.into_parts();
    let credential = match sdk_credential(&parts.headers) {
        Ok(value) => value,
        Err(error) => return error_response(format, error),
    };
    let idempotency_key = header_text(&parts.headers, "idempotency-key").unwrap_or_default();
    let payload = match bounded_body(body, &parts.headers, state.limits.maximum_payload_bytes).await
    {
        Ok(value) => value,
        Err(error) => return error_response(format, error),
    };
    match state
        .service
        .accept(Candidate {
            kind,
            format,
            payload,
            credential,
            idempotency_key,
            replay: None,
        })
        .await
    {
        Ok(receipt) => success_response(format, &receipt),
        Err(error) => error_response(format, error),
    }
}

async fn bounded_body(
    body: Body,
    headers: &HeaderMap,
    maximum: usize,
) -> Result<Vec<u8>, AdmissionError> {
    let compressed = to_bytes(body, maximum + 1).await.map_err(|error| {
        AdmissionError::with_source(
            ErrorCode::TooLarge,
            "compressed request payload exceeds the configured limit",
            error,
        )
    })?;
    let encoding = header_text(headers, header::CONTENT_ENCODING.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if encoding.is_empty() || encoding == "identity" {
        return Ok(compressed.to_vec());
    }
    if encoding != "gzip" {
        return Err(AdmissionError::new(
            ErrorCode::Invalid,
            "unsupported content encoding",
        ));
    }
    let limit = u64::try_from(maximum + 1).map_err(|_| {
        AdmissionError::new(ErrorCode::TooLarge, "request payload limit is invalid")
    })?;
    let mut decoder = GzDecoder::new(compressed.as_ref()).take(limit);
    let mut payload = Vec::new();
    decoder.read_to_end(&mut payload).map_err(|error| {
        AdmissionError::with_source(ErrorCode::Invalid, "gzip request body is malformed", error)
    })?;
    if payload.len() > maximum {
        return Err(AdmissionError::new(
            ErrorCode::TooLarge,
            "request payload exceeds the configured limit",
        ));
    }
    Ok(payload)
}

fn otlp_format(headers: &HeaderMap) -> Result<PayloadFormat, AdmissionError> {
    let media_type = media_type_header(headers);
    match media_type.as_str() {
        "application/x-protobuf" => Ok(PayloadFormat::Protobuf),
        "application/json" => Ok(PayloadFormat::Json),
        _ => Err(AdmissionError::new(
            ErrorCode::Invalid,
            "unsupported OTLP content type",
        )),
    }
}

fn media_type_header(headers: &HeaderMap) -> String {
    header_text(headers, header::CONTENT_TYPE.as_str())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn sdk_credential(headers: &HeaderMap) -> Result<String, AdmissionError> {
    let mut values = Vec::new();
    for value in headers.get_all(header::AUTHORIZATION) {
        let value = value.to_str().map_err(|_| unauthorized())?;
        let credential = value.strip_prefix("Bearer ").ok_or_else(unauthorized)?;
        if credential.is_empty() {
            return Err(unauthorized());
        }
        values.push(credential.to_owned());
    }
    for value in headers.get_all("x-chill-sdk-key") {
        let value = value.to_str().map_err(|_| unauthorized())?;
        if value.is_empty() {
            return Err(unauthorized());
        }
        values.push(value.to_owned());
    }
    if values.len() == 1 {
        values.pop().ok_or_else(unauthorized)
    } else {
        Err(unauthorized())
    }
}

fn success_response(format: PayloadFormat, receipt: &Receipt) -> Response {
    let body = if format == PayloadFormat::Protobuf {
        Vec::new()
    } else {
        b"{}".to_vec()
    };
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(media_type(format)),
    );
    receipt_headers(response.headers_mut(), receipt);
    response
}

fn replay_success(receipt: &Receipt) -> Response {
    let mut response = axum::Json(serde_json::json!({
        "duplicate": receipt.duplicate,
        "idempotencyKey": receipt.idempotency_key,
        "inboxId": receipt.inbox_id,
        "serverReceivedAt": receipt.server_received_at.to_string(),
    }))
    .into_response();
    receipt_headers(response.headers_mut(), receipt);
    response
}

fn error_response(format: PayloadFormat, error: AdmissionError) -> Response {
    let (status, code) = status_codes(error.code);
    let message = error.message;
    let body = if format == PayloadFormat::Protobuf {
        RpcStatus {
            code,
            message: message.clone(),
        }
        .encode_to_vec()
    } else {
        serde_json::to_vec(&serde_json::json!({"code": code, "message": message}))
            .unwrap_or_default()
    };
    let mut response = (status, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(media_type(format)),
    );
    if let Some(retry_after) = error.retry_after
        && let Ok(value) = HeaderValue::try_from(retry_after.as_secs().max(1).to_string())
    {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

fn receipt_headers(headers: &mut HeaderMap, receipt: &Receipt) {
    for (name, value) in [
        ("idempotency-key", receipt.idempotency_key.clone()),
        ("chill-inbox-id", receipt.inbox_id.to_string()),
        ("chill-duplicate", receipt.duplicate.to_string()),
    ] {
        if let Ok(value) = HeaderValue::try_from(value) {
            headers.insert(name, value);
        }
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn replay_metadata(headers: &HeaderMap) -> Result<ReplayMetadata, AdmissionError> {
    let parse_nano = |name: &str| {
        required_header(headers, name)?
            .parse::<u64>()
            .map_err(|_| replay_headers_invalid())
    };
    Ok(ReplayMetadata {
        replay_id: required_header(headers, "chill-replay-id")?,
        chunk_id: required_header(headers, "chill-replay-chunk-id")?,
        session_id: required_header(headers, "chill-replay-session-id")?,
        boot_id: required_header(headers, "chill-replay-boot-id")?,
        start_monotonic_nano: parse_nano("chill-replay-start-monotonic-nano")?,
        end_monotonic_nano: parse_nano("chill-replay-end-monotonic-nano")?,
        occurred_at_unix_nano: parse_nano("chill-replay-occurred-at-unix-nano")?,
        digest: required_header(headers, "chill-replay-digest")?,
        codec: required_header(headers, "chill-replay-codec")?,
        traceparent: header_text(headers, "traceparent").filter(|value| !value.is_empty()),
    })
}

fn required_header(headers: &HeaderMap, name: &str) -> Result<String, AdmissionError> {
    header_text(headers, name)
        .filter(|value| !value.is_empty())
        .ok_or_else(replay_headers_invalid)
}

const fn media_type(format: PayloadFormat) -> &'static str {
    if matches!(format, PayloadFormat::Protobuf) {
        "application/x-protobuf"
    } else {
        "application/json"
    }
}

const fn status_codes(code: ErrorCode) -> (StatusCode, i32) {
    match code {
        ErrorCode::Invalid => (StatusCode::BAD_REQUEST, 3),
        ErrorCode::Unauthorized => (StatusCode::UNAUTHORIZED, 16),
        ErrorCode::Forbidden => (StatusCode::FORBIDDEN, 7),
        ErrorCode::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, 8),
        ErrorCode::Conflict => (StatusCode::CONFLICT, 9),
        ErrorCode::Quota | ErrorCode::Backpressure => (StatusCode::TOO_MANY_REQUESTS, 8),
        ErrorCode::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, 14),
    }
}

fn unauthorized() -> AdmissionError {
    AdmissionError::new(ErrorCode::Unauthorized, "SDK key is invalid")
}

fn replay_headers_invalid() -> AdmissionError {
    AdmissionError::new(ErrorCode::Invalid, "replay metadata headers are invalid")
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[derive(Clone, PartialEq, prost::Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
}
