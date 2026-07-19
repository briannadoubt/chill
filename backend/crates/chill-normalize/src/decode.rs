use std::collections::BTreeMap;

use opentelemetry_proto::tonic::{
    collector::{
        logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
        trace::v1::ExportTraceServiceRequest,
    },
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
    logs::v1::LogRecord,
};
use prost::Message;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{InboxItem, Record, TimingPolicy};

const BEHAVIOR_SCHEMA: &str = "https://schemas.chill.dev/behavior/v1/envelope.schema.json";

/// Fail-closed canonical decoding error.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// The durable payload or trusted metadata is malformed.
    #[error("invalid normalization payload: {0}")]
    Invalid(String),
    /// Protobuf decoding failed.
    #[error("decode protobuf: {0}")]
    Protobuf(#[from] prost::DecodeError),
    /// JSON decoding or canonical serialization failed.
    #[error("decode JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Deterministic canonical decoder.
#[derive(Clone, Copy, Debug)]
pub struct Decoder {
    timing: TimingPolicy,
}

impl Decoder {
    /// Creates a decoder using the supplied timing policy.
    #[must_use]
    pub const fn new(timing: TimingPolicy) -> Self {
        Self { timing }
    }

    /// Decodes one inbox payload into stable canonical envelopes.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed OTLP, replay metadata, or behavior contracts.
    pub fn decode(self, item: &InboxItem) -> Result<Vec<Record>, DecodeError> {
        match item.signal_kind.as_str() {
            "logs" => self.decode_logs(item),
            "traces" => self.decode_traces(item),
            "metrics" => self.decode_metrics(item),
            "replay" => self.decode_replay(item),
            kind => Err(invalid(format!("unsupported signal kind {kind:?}"))),
        }
    }

    fn decode_logs(self, item: &InboxItem) -> Result<Vec<Record>, DecodeError> {
        let request: ExportLogsServiceRequest = decode_message(item)?;
        let mut records = Vec::new();
        for resource_logs in request.resource_logs {
            let resource = attributes(
                resource_logs
                    .resource
                    .as_ref()
                    .map_or(&[][..], |value| value.attributes.as_slice()),
            )?;
            for scope_logs in resource_logs.scope_logs {
                for log in scope_logs.log_records {
                    let ordinal = ordinal(records.len())?;
                    let values = attributes(&log.attributes)?;
                    if values.contains_key("chill.schema.version") {
                        records.push(self.behavior(
                            item,
                            ordinal,
                            &resource,
                            scope_logs.scope.as_ref(),
                            &scope_logs.schema_url,
                            &log,
                            values,
                        )?);
                    } else {
                        records.push(self.generic_log(
                            item,
                            ordinal,
                            &resource,
                            scope_logs.scope.as_ref(),
                            &scope_logs.schema_url,
                            &log,
                        ));
                    }
                }
            }
        }
        Ok(records)
    }

    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "canonical behavior contract is reviewed as one deterministic transformation"
    )]
    fn behavior(
        self,
        item: &InboxItem,
        ordinal: i32,
        resource: &Map<String, Value>,
        scope: Option<&InstrumentationScope>,
        scope_schema: &str,
        log: &LogRecord,
        mut values: Map<String, Value>,
    ) -> Result<Record, DecodeError> {
        let identity = Value::Object(values.clone());
        let schema_version = required_string(&mut values, "chill.schema.version")?;
        let schema_url = required_string(&mut values, "chill.schema.url")?;
        let record_id = required_string(&mut values, "chill.record.id")?;
        let subject_id = required_string(&mut values, "chill.subject.id")?;
        let kind = required_string(&mut values, "chill.behavior.kind")?;
        let operation = required_string(&mut values, "chill.behavior.operation")?;
        let name = required_string(&mut values, "chill.behavior.name")?;
        if schema_version != "1.0.0" || schema_url != BEHAVIOR_SCHEMA {
            return Err(invalid("unsupported behavior schema"));
        }
        let uuid_v7 =
            Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
                .map_err(|error| invalid(error.to_string()))?;
        if !uuid_v7.is_match(&record_id) || !uuid_v7.is_match(&subject_id) {
            return Err(invalid(
                "behavior record and subject IDs must be lowercase UUIDv7",
            ));
        }
        if matches!(kind.as_str(), "action" | "impression" | "event") && record_id != subject_id {
            return Err(invalid("instant behavior subject must equal record ID"));
        }
        for derived in [
            "app.screen.id",
            "app.screen.name",
            "app.widget.id",
            "session.id",
            "chill.otel.semconv.version",
        ] {
            values.remove(derived);
        }
        let mut sections: BTreeMap<&str, Map<String, Value>> = [
            "source", "actor", "context", "privacy", "outcome", "payload",
        ]
        .into_iter()
        .map(|name| (name, Map::new()))
        .collect();
        let mut annotations = Map::new();
        let mut classifications = Map::new();
        let mut clock = Map::from_iter([
            (
                "occurred_at_unix_nano".to_owned(),
                Value::String(log.time_unix_nano.to_string()),
            ),
            (
                "observed_at_unix_nano".to_owned(),
                Value::String(log.observed_time_unix_nano.to_string()),
            ),
        ]);
        let mut trace_extras = Map::new();
        let mut duration = None;
        for (key, value) in values {
            if let Some(name) = key.strip_prefix("chill.privacy.annotation_classification.") {
                assign_literal(&mut classifications, name, value)?;
            } else if let Some(name) = key.strip_prefix("chill.annotation.") {
                assign_literal(&mut annotations, name, value)?;
            } else if let Some(path) = key.strip_prefix("chill.clock.") {
                assign(&mut clock, path, value)?;
            } else if let Some(path) = key.strip_prefix("chill.trace.") {
                assign(&mut trace_extras, path, value)?;
            } else if key == "chill.duration_nano" {
                duration = Some(value);
            } else if key.starts_with("chill.tenant.") {
                return Err(invalid("client-supplied tenant attributes are forbidden"));
            } else {
                let mut matched = false;
                for (section, target) in &mut sections {
                    if let Some(path) = key.strip_prefix(&format!("chill.{section}.")) {
                        assign(target, path, value.clone())?;
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return Err(invalid(format!("unknown Chill OTLP attribute {key:?}")));
                }
            }
        }
        sections
            .get_mut("privacy")
            .ok_or_else(|| invalid("missing privacy section"))?
            .insert(
                "annotation_classifications".to_owned(),
                Value::Object(classifications),
            );
        let installation_id = section_string(&sections, "source", "installation_id")?;
        let session_id = section_string(&sections, "context", "session_id")?;
        if !source_platform_is_supported(&section_string(&sections, "source", "platform")?) {
            return Err(invalid(
                "behavior source platform is missing or unsupported",
            ));
        }
        let sequence = unsigned(clock.get("sequence_number"), "sequence_number")?;
        if sequence > 9_007_199_254_740_991 {
            return Err(invalid("clock sequence_number is invalid"));
        }
        clock.insert("sequence_number".to_owned(), Value::Number(sequence.into()));
        let monotonic = optional_unsigned(clock.get("monotonic_nano"))?;
        let boot_id = clock
            .get("boot_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if monotonic.is_some() != boot_id.is_some() {
            return Err(invalid(
                "clock monotonic_nano and boot_id must appear together",
            ));
        }
        if let Some(value) = monotonic {
            clock.insert(
                "monotonic_nano".to_owned(),
                Value::String(value.to_string()),
            );
        }
        if operation != "end" && duration.is_some() {
            return Err(invalid("duration_nano is valid only for end records"));
        }
        let occurred = log.time_unix_nano;
        let observed = log.observed_time_unix_nano;
        let timing = self
            .timing
            .classify(item.server_received_at, Some(occurred));
        clock.insert(
            "observed_at_unix_nano".to_owned(),
            Value::String(timing.canonical_observed.to_string()),
        );
        let mut canonical = Map::from_iter([
            ("schema_version".to_owned(), Value::String(schema_version)),
            ("schema_url".to_owned(), Value::String(schema_url)),
            ("record_id".to_owned(), Value::String(record_id.clone())),
            ("subject_id".to_owned(), Value::String(subject_id)),
            ("kind".to_owned(), Value::String(kind)),
            ("operation".to_owned(), Value::String(operation)),
            ("name".to_owned(), Value::String(name)),
            ("tenant".to_owned(), tenant(item)),
            (
                "source".to_owned(),
                Value::Object(take_section(&mut sections, "source")?),
            ),
            ("clock".to_owned(), Value::Object(clock)),
            (
                "instrumentation".to_owned(),
                instrumentation(scope, scope_schema),
            ),
            ("resource".to_owned(), json!({"attributes": resource})),
            ("annotations".to_owned(), Value::Object(annotations)),
            (
                "privacy".to_owned(),
                Value::Object(take_section(&mut sections, "privacy")?),
            ),
            (
                "payload".to_owned(),
                Value::Object(take_section(&mut sections, "payload")?),
            ),
        ]);
        for section in ["actor", "context", "outcome"] {
            let value = take_section(&mut sections, section)?;
            if !value.is_empty() {
                canonical.insert(section.to_owned(), Value::Object(value));
            }
        }
        if let Some(value) = duration {
            canonical.insert("duration_nano".to_owned(), value);
        }
        let (trace_id, span_id) = append_trace(&mut canonical, log, trace_extras)?;
        let digest = Sha256::digest(serde_json::to_vec(&json!({
            "attributes": identity,
            "time": occurred,
            "observed": observed,
            "trace_id": hex::encode(&log.trace_id),
            "span_id": hex::encode(&log.span_id),
        }))?);
        Ok(Record {
            ordinal,
            envelope_kind: "behavior".to_owned(),
            record_id,
            digest: digest.into(),
            installation_id: Some(installation_id),
            session_id: Some(session_id),
            replay_id: nested_string(&canonical, "context", "replay_id"),
            replay_chunk_id: nested_string(&canonical, "payload", "chunk_id"),
            trace_id,
            span_id,
            occurred_at_unix_nano: Some(occurred),
            source_observed_at_unix_nano: Some(observed),
            monotonic_nano: monotonic,
            boot_id,
            sequence_number: Some(sequence),
            timing,
            canonical: Value::Object(canonical),
        })
    }

    fn generic_log(
        self,
        item: &InboxItem,
        ordinal: i32,
        resource: &Map<String, Value>,
        scope: Option<&InstrumentationScope>,
        scope_schema: &str,
        log: &LogRecord,
    ) -> Record {
        let body = log.encode_to_vec();
        let source = (log.time_unix_nano != 0).then_some(log.time_unix_nano);
        let timing = self.timing.classify(item.server_received_at, source);
        let trace_id = sized_hex(&log.trace_id, 16);
        let span_id = sized_hex(&log.span_id, 8);
        base_record(
            item,
            ordinal,
            "otel.log",
            generated_id("log", item, ordinal, &body),
            &body,
            trace_id,
            span_id,
            source,
            timing,
            json!({
                "resource": {"attributes": resource},
                "instrumentation": instrumentation(scope, scope_schema),
                "log_record": log,
            }),
        )
    }

    fn decode_traces(self, item: &InboxItem) -> Result<Vec<Record>, DecodeError> {
        let request: ExportTraceServiceRequest = decode_message(item)?;
        let mut records = Vec::new();
        for resource_spans in request.resource_spans {
            let resource = attributes(
                resource_spans
                    .resource
                    .as_ref()
                    .map_or(&[][..], |value| value.attributes.as_slice()),
            )?;
            for scope_spans in resource_spans.scope_spans {
                for span in scope_spans.spans {
                    let ordinal = ordinal(records.len())?;
                    let body = span.encode_to_vec();
                    let trace_id = hex::encode(&span.trace_id);
                    let span_id = hex::encode(&span.span_id);
                    let source = Some(span.start_time_unix_nano);
                    let timing = self.timing.classify(item.server_received_at, source);
                    let record = base_record(
                        item,
                        ordinal,
                        "otel.span",
                        format!("span:{trace_id}:{span_id}"),
                        &body,
                        Some(trace_id),
                        Some(span_id),
                        source,
                        timing,
                        json!({"resource":{"attributes":resource}, "instrumentation":instrumentation(scope_spans.scope.as_ref(), &scope_spans.schema_url), "span":span}),
                    );
                    records.push(record);
                }
            }
        }
        Ok(records)
    }

    fn decode_metrics(self, item: &InboxItem) -> Result<Vec<Record>, DecodeError> {
        let request: ExportMetricsServiceRequest = decode_message(item)?;
        let mut records = Vec::new();
        for resource_metrics in request.resource_metrics {
            let resource = attributes(
                resource_metrics
                    .resource
                    .as_ref()
                    .map_or(&[][..], |value| value.attributes.as_slice()),
            )?;
            for scope_metrics in resource_metrics.scope_metrics {
                for metric in scope_metrics.metrics {
                    let ordinal = ordinal(records.len())?;
                    let body = metric.encode_to_vec();
                    let timing = self.timing.classify(item.server_received_at, None);
                    records.push(base_record(
                        item,
                        ordinal,
                        "otel.metric",
                        generated_id("metric", item, ordinal, &body),
                        &body,
                        None,
                        None,
                        None,
                        timing,
                        json!({"resource":{"attributes":resource}, "instrumentation":instrumentation(scope_metrics.scope.as_ref(), &scope_metrics.schema_url), "metric":metric}),
                    ));
                }
            }
        }
        Ok(records)
    }

    fn decode_replay(self, item: &InboxItem) -> Result<Vec<Record>, DecodeError> {
        let replay = item
            .metadata
            .get("replay")
            .ok_or_else(|| invalid("trusted replay metadata is missing"))?;
        let string = |name: &str| {
            replay
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| invalid(format!("replay {name} is missing")))
        };
        let number = |name: &str| {
            replay
                .get(name)
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid(format!("replay {name} is missing")))
        };
        let replay_id = string("replay_id")?;
        let chunk_id = string("chunk_id")?;
        let session_id = string("session_id")?;
        let boot_id = string("boot_id")?;
        let start = number("start_monotonic_nano")?;
        let end = number("end_monotonic_nano")?;
        let occurred = number("occurred_at_unix_nano")?;
        let digest_text = string("digest")?;
        let digest: [u8; 32] = hex::decode(&digest_text)
            .map_err(|_| invalid("replay digest is invalid"))?
            .try_into()
            .map_err(|_| invalid("replay digest is invalid"))?;
        let timing = self
            .timing
            .classify(item.server_received_at, Some(occurred));
        let traceparent = replay.get("traceparent").and_then(Value::as_str);
        let trace_parts = traceparent.map(|value| value.split('-').collect::<Vec<_>>());
        let trace_id = trace_parts
            .as_ref()
            .and_then(|parts| parts.get(1))
            .map(ToString::to_string);
        let span_id = trace_parts
            .as_ref()
            .and_then(|parts| parts.get(2))
            .map(ToString::to_string);
        let record_id = format!("replay:{chunk_id}");
        let canonical = json!({
            "schema_version":"1.0.0", "envelope_kind":"replay", "record_id":record_id,
            "tenant":tenant(item), "session_id":session_id, "replay_id":replay_id,
            "chunk_id":chunk_id, "boot_id":boot_id, "start_monotonic_nano":start.to_string(),
            "end_monotonic_nano":end.to_string(), "occurred_at_unix_nano":occurred.to_string(),
            "server_received_at_unix_nano":timing.server_received_nano.to_string(),
            "digest":digest_text, "byte_count":item.payload.len(), "codec":string("codec")?,
            "trace_id":trace_id, "span_id":span_id,
        });
        Ok(vec![Record {
            ordinal: 0,
            envelope_kind: "replay".to_owned(),
            record_id,
            digest,
            installation_id: None,
            session_id: Some(session_id),
            replay_id: Some(replay_id),
            replay_chunk_id: Some(chunk_id),
            trace_id,
            span_id,
            occurred_at_unix_nano: Some(occurred),
            source_observed_at_unix_nano: None,
            monotonic_nano: Some(start),
            boot_id: Some(boot_id),
            sequence_number: None,
            timing,
            canonical,
        }])
    }
}

fn decode_message<T>(item: &InboxItem) -> Result<T, DecodeError>
where
    T: Message + Default + DeserializeOwned,
{
    match item.payload_format.as_str() {
        "protobuf" => T::decode(item.payload.as_slice()).map_err(Into::into),
        "json" => serde_json::from_slice(&item.payload).map_err(Into::into),
        format => Err(invalid(format!("unsupported payload format {format:?}"))),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "record constructor mirrors canonical table fields"
)]
fn base_record(
    item: &InboxItem,
    ordinal: i32,
    kind: &str,
    record_id: String,
    body: &[u8],
    trace_id: Option<String>,
    span_id: Option<String>,
    occurred: Option<u64>,
    timing: crate::Timing,
    content: Value,
) -> Record {
    let mut canonical = Map::from_iter([
        (
            "schema_version".to_owned(),
            Value::String("1.0.0".to_owned()),
        ),
        ("envelope_kind".to_owned(), Value::String(kind.to_owned())),
        ("record_id".to_owned(), Value::String(record_id.clone())),
        ("tenant".to_owned(), tenant(item)),
        (
            "server_received_at_unix_nano".to_owned(),
            Value::String(timing.server_received_nano.to_string()),
        ),
    ]);
    if let Value::Object(content) = content {
        canonical.extend(content);
    }
    Record {
        ordinal,
        envelope_kind: kind.to_owned(),
        record_id,
        digest: Sha256::digest(body).into(),
        installation_id: None,
        session_id: None,
        replay_id: None,
        replay_chunk_id: None,
        trace_id,
        span_id,
        occurred_at_unix_nano: occurred,
        source_observed_at_unix_nano: None,
        monotonic_nano: None,
        boot_id: None,
        sequence_number: None,
        timing,
        canonical: Value::Object(canonical),
    }
}

fn attributes(values: &[KeyValue]) -> Result<Map<String, Value>, DecodeError> {
    let mut result = Map::new();
    for entry in values {
        if entry.key.is_empty() || result.contains_key(&entry.key) {
            return Err(invalid("OTLP attribute keys must be non-empty and unique"));
        }
        let value = entry
            .value
            .as_ref()
            .ok_or_else(|| invalid("OTLP attribute value is missing"))?;
        result.insert(entry.key.clone(), any_value(value)?);
    }
    Ok(result)
}

fn any_value(value: &AnyValue) -> Result<Value, DecodeError> {
    match value.value.as_ref() {
        Some(any_value::Value::StringValue(value)) => Ok(Value::String(value.clone())),
        Some(any_value::Value::BoolValue(value)) => Ok(Value::Bool(*value)),
        Some(any_value::Value::IntValue(value)) => Ok(Value::Number((*value).into())),
        Some(any_value::Value::DoubleValue(value)) => serde_json::Number::from_f64(*value)
            .map(Value::Number)
            .ok_or_else(|| invalid("OTLP double must be finite")),
        Some(any_value::Value::BytesValue(value)) => Ok(Value::String(hex::encode(value))),
        Some(any_value::Value::ArrayValue(value)) => value.values.iter().map(any_value).collect(),
        Some(any_value::Value::KvlistValue(value)) => attributes(&value.values).map(Value::Object),
        Some(any_value::Value::StringValueStrindex(_)) => {
            Err(invalid("OTLP string dictionary indexes are unsupported"))
        }
        None => Err(invalid("OTLP AnyValue is empty")),
    }
}

fn required_string(values: &mut Map<String, Value>, key: &str) -> Result<String, DecodeError> {
    values
        .remove(key)
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("{key} is required")))
}

fn section_string(
    sections: &BTreeMap<&str, Map<String, Value>>,
    section: &str,
    key: &str,
) -> Result<String, DecodeError> {
    sections
        .get(section)
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("{section} {key} is required")))
}

fn nested_string(canonical: &Map<String, Value>, section: &str, key: &str) -> Option<String> {
    canonical
        .get(section)
        .and_then(Value::as_object)
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn take_section(
    sections: &mut BTreeMap<&str, Map<String, Value>>,
    key: &str,
) -> Result<Map<String, Value>, DecodeError> {
    sections
        .remove(key)
        .ok_or_else(|| invalid(format!("missing {key} section")))
}

fn unsigned(value: Option<&Value>, name: &str) -> Result<u64, DecodeError> {
    optional_unsigned(value)?.ok_or_else(|| invalid(format!("{name} is required")))
}

fn optional_unsigned(value: Option<&Value>) -> Result<Option<u64>, DecodeError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| invalid("unsigned value is invalid")),
        Some(Value::String(text)) => text
            .parse::<u64>()
            .map(Some)
            .map_err(|_| invalid("unsigned value is invalid")),
        Some(_) => Err(invalid("unsigned value is invalid")),
    }
}

fn assign(target: &mut Map<String, Value>, path: &str, value: Value) -> Result<(), DecodeError> {
    let mut parts = path.split('.').peekable();
    let mut cursor = target;
    while let Some(part) = parts.next() {
        if part.is_empty() {
            return Err(invalid("flattened attribute path is invalid"));
        }
        if parts.peek().is_none() {
            if cursor.insert(part.to_owned(), value).is_some() {
                return Err(invalid("duplicate flattened attribute path"));
            }
            return Ok(());
        }
        let child = cursor
            .entry(part.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        cursor = child
            .as_object_mut()
            .ok_or_else(|| invalid("flattened attribute path collision"))?;
    }
    Err(invalid("flattened attribute path is empty"))
}

fn assign_literal(
    target: &mut Map<String, Value>,
    name: &str,
    value: Value,
) -> Result<(), DecodeError> {
    if name.is_empty() {
        return Err(invalid("annotation name is empty"));
    }
    if target.insert(name.to_owned(), value).is_some() {
        return Err(invalid("duplicate annotation name"));
    }
    Ok(())
}

fn append_trace(
    canonical: &mut Map<String, Value>,
    log: &LogRecord,
    mut extras: Map<String, Value>,
) -> Result<(Option<String>, Option<String>), DecodeError> {
    if log.trace_id.is_empty() {
        if !log.span_id.is_empty() {
            return Err(invalid("span ID without trace ID"));
        }
        return Ok((None, None));
    }
    let trace = sized_hex(&log.trace_id, 16).ok_or_else(|| invalid("invalid trace ID"))?;
    if trace.bytes().all(|byte| byte == b'0') {
        return Err(invalid("invalid trace ID"));
    }
    let span = if log.span_id.is_empty() {
        None
    } else {
        Some(sized_hex(&log.span_id, 8).ok_or_else(|| invalid("invalid span ID"))?)
    };
    extras.insert("trace_id".to_owned(), Value::String(trace.clone()));
    if let Some(value) = &span {
        extras.insert("span_id".to_owned(), Value::String(value.clone()));
    }
    extras.insert(
        "trace_flags".to_owned(),
        Value::String(format!("{:02x}", log.flags & 0xff)),
    );
    canonical.insert("trace".to_owned(), Value::Object(extras));
    Ok((Some(trace), span))
}

fn generated_id(kind: &str, item: &InboxItem, ordinal: i32, body: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(item.data_source_id.as_bytes());
    digest.update([0]);
    digest.update(ordinal.to_string().as_bytes());
    digest.update([0]);
    digest.update(body);
    format!("{kind}:{}", hex::encode(digest.finalize()))
}

fn sized_hex(value: &[u8], size: usize) -> Option<String> {
    (value.len() == size).then(|| hex::encode(value))
}

fn instrumentation(scope: Option<&InstrumentationScope>, schema_url: &str) -> Value {
    let mut result = json!({"name":scope.map_or("", |value| value.name.as_str()), "version":scope.map_or("", |value| value.version.as_str())});
    if !schema_url.is_empty() {
        result["schema_url"] = Value::String(schema_url.to_owned());
    }
    result
}

fn tenant(item: &InboxItem) -> Value {
    json!({"organization_id":item.organization_id, "project_id":item.project_id, "environment_id":item.environment_id})
}

fn ordinal(value: usize) -> Result<i32, DecodeError> {
    i32::try_from(value).map_err(|_| invalid("record count exceeds canonical ordinal range"))
}

fn invalid(message: impl Into<String>) -> DecodeError {
    DecodeError::Invalid(message.into())
}

fn source_platform_is_supported(value: &str) -> bool {
    matches!(value, "apple" | "android" | "web" | "server")
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, json};

    use super::{DecodeError, assign, assign_literal, source_platform_is_supported};

    #[test]
    fn every_canonical_source_platform_is_supported() {
        for platform in ["apple", "android", "web", "server"] {
            assert!(source_platform_is_supported(platform));
        }
        assert!(!source_platform_is_supported("browser"));
    }

    #[test]
    fn dotted_annotation_names_are_literal_keys() -> Result<(), DecodeError> {
        let mut annotations = Map::new();
        assign_literal(&mut annotations, "cat.id", json!("cat_12345"))?;

        assert_eq!(
            annotations,
            Map::from_iter([("cat.id".to_owned(), json!("cat_12345"))])
        );
        Ok(())
    }

    #[test]
    fn dotted_section_paths_remain_nested() -> Result<(), DecodeError> {
        let mut section = Map::new();
        assign(&mut section, "page.surface_id", json!("surface"))?;

        assert_eq!(
            section,
            Map::from_iter([("page".to_owned(), json!({"surface_id": "surface"}))])
        );
        Ok(())
    }

    #[test]
    fn duplicate_literal_annotation_names_are_rejected() -> Result<(), DecodeError> {
        let mut annotations = Map::new();
        assign_literal(&mut annotations, "cat.id", json!("first"))?;

        assert!(assign_literal(&mut annotations, "cat.id", json!("second")).is_err());
        Ok(())
    }
}
