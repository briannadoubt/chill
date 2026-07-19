use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::LazyLock,
};

use opentelemetry_proto::tonic::{
    collector::{
        logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
        trace::v1::ExportTraceServiceRequest,
    },
    common::v1::{KeyValue, any_value},
};
use prost::Message;
use regex::Regex;
use sha2::{Digest as _, Sha256};

use crate::{
    AdmissionError, Candidate, ErrorCode, Limits, PayloadFormat, ReplayMetadata, SchemaRef,
    SignalKind, ValidatedRequest,
};

static SAFE_IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
        .unwrap_or_else(|error| unreachable!("static safe-identifier regex is invalid: {error}"))
});
static TRACEPARENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^00-[0-9a-f]{32}-[0-9a-f]{16}-0[01]$")
        .unwrap_or_else(|error| unreachable!("static traceparent regex is invalid: {error}"))
});

/// Validates a bounded transport candidate and derives its idempotency identity.
///
/// # Errors
///
/// Returns a stable admission error for malformed, empty, oversized, or unsafe input.
pub fn validate_candidate(
    candidate: Candidate,
    limits: Limits,
) -> Result<ValidatedRequest, AdmissionError> {
    limits.validate()?;
    let maximum_bytes = if candidate.kind == SignalKind::Replay {
        limits.maximum_replay_bytes
    } else {
        limits.maximum_payload_bytes
    };
    if candidate.payload.is_empty() {
        return Err(AdmissionError::new(
            ErrorCode::Invalid,
            "request payload is empty",
        ));
    }
    if candidate.payload.len() > maximum_bytes {
        return Err(AdmissionError::new(
            ErrorCode::TooLarge,
            "request payload exceeds the configured limit",
        ));
    }
    let payload_digest = Sha256::digest(&candidate.payload);
    let (record_count, schema_refs, record_ids) = match candidate.kind {
        SignalKind::Logs => {
            validate_logs(candidate.format, &candidate.payload, limits.maximum_records)
                .map_err(AdmissionError::invalid)?
        }
        SignalKind::Traces => (
            validate_traces(candidate.format, &candidate.payload, limits.maximum_records)
                .map_err(AdmissionError::invalid)?,
            Vec::new(),
            Vec::new(),
        ),
        SignalKind::Metrics => (
            validate_metrics(candidate.format, &candidate.payload, limits.maximum_records)
                .map_err(AdmissionError::invalid)?,
            Vec::new(),
            Vec::new(),
        ),
        SignalKind::Replay => {
            validate_replay(
                candidate.replay.as_ref(),
                &candidate.payload,
                &payload_digest,
            )
            .map_err(AdmissionError::invalid)?;
            (1, Vec::new(), Vec::new())
        }
    };
    let idempotency_key = normalize_idempotency(
        &candidate.idempotency_key,
        candidate.replay.as_ref(),
        &record_ids,
        &payload_digest,
    )?;
    Ok(ValidatedRequest {
        kind: candidate.kind,
        format: candidate.format,
        payload: candidate.payload,
        payload_digest,
        idempotency_key,
        record_count,
        schema_refs,
        replay: candidate.replay,
    })
}

fn validate_logs(
    format: PayloadFormat,
    payload: &[u8],
    maximum_records: usize,
) -> Result<(usize, Vec<SchemaRef>, Vec<String>), ValidationError> {
    let request: ExportLogsServiceRequest = decode(format, payload)?;
    let mut count = 0_usize;
    let mut schemas = BTreeSet::new();
    let mut record_ids = BTreeSet::new();
    for resource in request.resource_logs {
        for scope in resource.scope_logs {
            let chill_scope = scope
                .scope
                .as_ref()
                .is_some_and(|value| value.name.starts_with("dev.chill."));
            for record in scope.log_records {
                count += 1;
                if count > maximum_records {
                    return Err(ValidationError::Message("log record limit exceeded"));
                }
                if !record.trace_id.is_empty() && record.trace_id.len() != 16 {
                    return Err(ValidationError::Message("trace ID must contain 16 bytes"));
                }
                if !record.span_id.is_empty() && record.span_id.len() != 8 {
                    return Err(ValidationError::Message("span ID must contain 8 bytes"));
                }
                let attributes = string_attributes(&record.attributes)?;
                if !chill_scope && !attributes.contains_key("chill.record.id") {
                    continue;
                }
                let record_id = bounded(&attributes, "chill.record.id", 128)?;
                if !SAFE_IDENTIFIER.is_match(record_id) || !record_ids.insert(record_id.to_owned())
                {
                    return Err(ValidationError::Message(
                        "Chill record ID is invalid or duplicated",
                    ));
                }
                schemas.insert(SchemaRef {
                    version: bounded(&attributes, "chill.schema.version", 64)?.to_owned(),
                    url: bounded(&attributes, "chill.schema.url", 2048)?.to_owned(),
                });
                if schemas.len() > 8 {
                    return Err(ValidationError::Message(
                        "request references more than 8 schemas",
                    ));
                }
            }
        }
    }
    if count == 0 {
        return Err(ValidationError::Message("request contains no log records"));
    }
    Ok((
        count,
        schemas.into_iter().collect(),
        record_ids.into_iter().collect(),
    ))
}

fn validate_traces(
    format: PayloadFormat,
    payload: &[u8],
    maximum_records: usize,
) -> Result<usize, ValidationError> {
    let request: ExportTraceServiceRequest = decode(format, payload)?;
    let mut count = 0_usize;
    for resource in request.resource_spans {
        for scope in resource.scope_spans {
            for span in scope.spans {
                count += 1;
                if count > maximum_records
                    || span.trace_id.len() != 16
                    || span.span_id.len() != 8
                    || span.name.is_empty()
                    || (span.end_time_unix_nano != 0
                        && span.end_time_unix_nano < span.start_time_unix_nano)
                {
                    return Err(ValidationError::Message("span shape is invalid"));
                }
            }
        }
    }
    if count == 0 {
        return Err(ValidationError::Message("request contains no spans"));
    }
    Ok(count)
}

fn validate_metrics(
    format: PayloadFormat,
    payload: &[u8],
    maximum_records: usize,
) -> Result<usize, ValidationError> {
    let request: ExportMetricsServiceRequest = decode(format, payload)?;
    let mut count = 0_usize;
    for resource in request.resource_metrics {
        for scope in resource.scope_metrics {
            for metric in scope.metrics {
                count += 1;
                if count > maximum_records || metric.name.is_empty() || metric.data.is_none() {
                    return Err(ValidationError::Message("metric shape is invalid"));
                }
            }
        }
    }
    if count == 0 {
        return Err(ValidationError::Message("request contains no metrics"));
    }
    Ok(count)
}

fn validate_replay(
    metadata: Option<&ReplayMetadata>,
    payload: &[u8],
    digest: &[u8],
) -> Result<(), ValidationError> {
    let metadata = metadata.ok_or(ValidationError::Message("replay metadata is required"))?;
    if [
        &metadata.replay_id,
        &metadata.chunk_id,
        &metadata.session_id,
        &metadata.boot_id,
    ]
    .into_iter()
    .any(|value| !SAFE_IDENTIFIER.is_match(value))
        || metadata.start_monotonic_nano == 0
        || metadata.end_monotonic_nano < metadata.start_monotonic_nano
        || metadata.occurred_at_unix_nano == 0
        || metadata.codec != "chill-json-gzip-aesgcm-v1"
        || payload.len() < 13
        || !payload.starts_with(b"CHILLRP1\n")
    {
        return Err(ValidationError::Message("replay envelope is invalid"));
    }
    let decoded = hex::decode(&metadata.digest).map_err(ValidationError::Hex)?;
    if metadata.digest != metadata.digest.to_lowercase() || decoded.as_slice() != digest {
        return Err(ValidationError::Message(
            "replay digest does not match payload",
        ));
    }
    if let Some(traceparent) = &metadata.traceparent {
        let parts: Vec<_> = traceparent.split('-').collect();
        if !TRACEPARENT.is_match(traceparent)
            || parts.get(1) == Some(&"00000000000000000000000000000000")
            || parts.get(2) == Some(&"0000000000000000")
        {
            return Err(ValidationError::Message("traceparent is invalid"));
        }
    }
    Ok(())
}

fn decode<T>(format: PayloadFormat, payload: &[u8]) -> Result<T, ValidationError>
where
    T: Message + Default + serde::de::DeserializeOwned,
{
    match format {
        PayloadFormat::Protobuf => T::decode(payload).map_err(ValidationError::Protobuf),
        PayloadFormat::Json => serde_json::from_slice(payload).map_err(ValidationError::Json),
        PayloadFormat::ReplayV1 => Err(ValidationError::Message("unsupported OTLP format")),
    }
}

fn string_attributes(attributes: &[KeyValue]) -> Result<HashMap<String, String>, ValidationError> {
    let mut values = HashMap::new();
    let mut seen = HashSet::new();
    for attribute in attributes {
        if attribute.key.is_empty() || attribute.value.is_none() || !seen.insert(&attribute.key) {
            return Err(ValidationError::Message(
                "log attributes are invalid or duplicated",
            ));
        }
        if let Some(any_value::Value::StringValue(value)) = attribute
            .value
            .as_ref()
            .and_then(|value| value.value.as_ref())
        {
            values.insert(attribute.key.clone(), value.clone());
        }
    }
    Ok(values)
}

fn bounded<'a>(
    attributes: &'a HashMap<String, String>,
    key: &str,
    maximum: usize,
) -> Result<&'a str, ValidationError> {
    attributes
        .get(key)
        .filter(|value| !value.is_empty() && value.len() <= maximum)
        .map(String::as_str)
        .ok_or(ValidationError::Message(
            "required Chill attribute is invalid",
        ))
}

fn normalize_idempotency(
    provided: &str,
    replay: Option<&ReplayMetadata>,
    record_ids: &[String],
    digest: &[u8],
) -> Result<String, AdmissionError> {
    if !provided.is_empty() {
        if SAFE_IDENTIFIER.is_match(provided) {
            return Ok(provided.to_owned());
        }
        return Err(AdmissionError::new(
            ErrorCode::Invalid,
            "idempotency key is invalid",
        ));
    }
    if let Some(replay) = replay {
        return Ok(format!("replay:{}", replay.chunk_id));
    }
    if !record_ids.is_empty() {
        return Ok(format!(
            "records:{}",
            hex::encode(Sha256::digest(record_ids.join("\0")))
        ));
    }
    Ok(format!("body:{}", hex::encode(digest)))
}

#[derive(Debug, thiserror::Error)]
enum ValidationError {
    #[error("{0}")]
    Message(&'static str),
    #[error("decode protobuf: {0}")]
    Protobuf(prost::DecodeError),
    #[error("decode JSON: {0}")]
    Json(serde_json::Error),
    #[error("decode hex: {0}")]
    Hex(hex::FromHexError),
}

#[cfg(test)]
mod tests {
    use opentelemetry_proto::tonic::{
        collector::logs::v1::ExportLogsServiceRequest,
        common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
        logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
    };

    use super::*;

    #[test]
    fn validates_chill_logs_and_stabilizes_record_idempotency() {
        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    scope: Some(InstrumentationScope {
                        name: "dev.chill.swift".to_owned(),
                        ..Default::default()
                    }),
                    log_records: vec![LogRecord {
                        attributes: vec![
                            string_attribute("chill.record.id", "record-1"),
                            string_attribute("chill.schema.version", "1.0.0"),
                            string_attribute("chill.schema.url", "https://schema.invalid/v1"),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let candidate = Candidate {
            kind: SignalKind::Logs,
            format: PayloadFormat::Protobuf,
            payload: request.encode_to_vec(),
            credential: "secret".to_owned(),
            idempotency_key: String::new(),
            replay: None,
        };
        let validated = validate_candidate(candidate, Limits::default())
            .unwrap_or_else(|error| unreachable!("valid fixture failed: {error}"));
        assert_eq!(validated.record_count, 1);
        assert!(validated.idempotency_key.starts_with("records:"));
        assert_eq!(validated.schema_refs.len(), 1);
    }

    fn string_attribute(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.to_owned(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(value.to_owned())),
            }),
            ..Default::default()
        }
    }
}
