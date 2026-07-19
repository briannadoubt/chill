"""Lossless Chill/OTLP mapping and safe W3C propagation reference.

The JSON objects use the normative OTLP/HTTP JSON field spelling. Trace and
span IDs are lowercase hex strings, enums are integers, and int64 values are
decimal strings. The mapper handles one record per request for clarity; native
exporters should batch records sharing resource and instrumentation scope.
"""

from __future__ import annotations

from dataclasses import dataclass
import re
from typing import Any, Iterable, Mapping
from urllib.parse import quote


JsonObject = dict[str, Any]

_TRACE_ID = re.compile(r"^(?!0{32}$)[0-9a-f]{32}$")
_SPAN_ID = re.compile(r"^(?!0{16}$)[0-9a-f]{16}$")
_BAGGAGE_KEY = re.compile(r"^[!#$%&'*+.^_`|~0-9A-Za-z-]+$")
_TRACESTATE_KEY = re.compile(
    r"^(?:[a-z][a-z0-9_\-*/]{0,255}|"
    r"[a-z0-9][a-z0-9_\-*/]{0,240}@[a-z][a-z0-9_\-*/]{0,13})$"
)
_ANNOTATION_PREFIX = "chill.annotation."
_ANNOTATION_CLASSIFICATION_PREFIX = "chill.privacy.annotation_classification."
_DERIVED_ATTRIBUTE_KEYS = {
    "app.screen.id",
    "app.screen.name",
    "app.widget.id",
    "session.id",
    "chill.otel.semconv.version",
}
_SEMCONV_VERSION = "1.43.0"

_SEVERITY_NUMBER = {
    "trace": 1,
    "debug": 5,
    "info": 9,
    "warn": 13,
    "error": 17,
    "fatal": 21,
}


@dataclass(frozen=True)
class SpanContext:
    trace_id: str
    span_id: str
    trace_flags: int = 0
    trace_state: str | None = None
    remote: bool = False

    def __post_init__(self) -> None:
        if not _TRACE_ID.fullmatch(self.trace_id):
            raise ValueError("invalid W3C trace ID")
        if not _SPAN_ID.fullmatch(self.span_id):
            raise ValueError("invalid W3C span ID")
        if not 0 <= self.trace_flags <= 255:
            raise ValueError("trace flags must fit in one byte")
        if self.trace_state is not None:
            _validate_trace_state(self.trace_state)


@dataclass(frozen=True)
class PropagationPolicy:
    trusted_origins: frozenset[str]
    baggage_allowlist: frozenset[str] = frozenset()
    max_baggage_entries: int = 8
    max_baggage_value_bytes: int = 128
    max_baggage_header_bytes: int = 1024

    def __post_init__(self) -> None:
        if self.max_baggage_entries < 0:
            raise ValueError("baggage entry limit must not be negative")
        if self.max_baggage_value_bytes < 0 or self.max_baggage_header_bytes < 0:
            raise ValueError("baggage byte limits must not be negative")


@dataclass(frozen=True)
class MessagingDecision:
    parent: SpanContext | None
    links: tuple[SpanContext, ...]


def _validate_trace_state(value: str) -> None:
    """Accept a strict canonical subset of the W3C tracestate grammar."""

    try:
        encoded = value.encode("ascii")
    except UnicodeEncodeError as error:
        raise ValueError("trace state must be ASCII") from error
    if not encoded or len(encoded) > 512:
        raise ValueError("trace state must contain 1 to 512 bytes")
    members = value.split(",")
    if len(members) > 32:
        raise ValueError("trace state exceeds 32 list members")
    seen: set[str] = set()
    for member in members:
        if member.count("=") != 1:
            raise ValueError("trace state member must contain one equals sign")
        key, member_value = member.split("=", 1)
        if not _TRACESTATE_KEY.fullmatch(key):
            raise ValueError("invalid trace state key")
        if key in seen:
            raise ValueError("duplicate trace state key")
        seen.add(key)
        if not 1 <= len(member_value) <= 256 or member_value.endswith(" "):
            raise ValueError("invalid trace state value length or trailing space")
        if any(
            not (
                0x20 <= ord(character) <= 0x2B
                or 0x2D <= ord(character) <= 0x3C
                or 0x3E <= ord(character) <= 0x7E
            )
            for character in member_value
        ):
            raise ValueError("invalid trace state value")


def _encode_any_value(value: object) -> JsonObject:
    if isinstance(value, bool):
        return {"boolValue": value}
    if isinstance(value, str):
        return {"stringValue": value}
    if isinstance(value, int):
        return {"intValue": str(value)}
    if isinstance(value, float):
        return {"doubleValue": value}
    if isinstance(value, list):
        return {
            "arrayValue": {
                "values": [_encode_any_value(item) for item in value],
            }
        }
    raise TypeError(f"unsupported OTLP attribute value: {type(value).__name__}")


def _decode_any_value(value: Mapping[str, object]) -> object:
    if len(value) != 1:
        raise ValueError("OTLP AnyValue must have exactly one populated field")
    kind, encoded = next(iter(value.items()))
    if kind == "boolValue" and isinstance(encoded, bool):
        return encoded
    if kind == "stringValue" and isinstance(encoded, str):
        return encoded
    if kind == "intValue" and isinstance(encoded, str):
        return int(encoded)
    if kind == "doubleValue" and isinstance(encoded, (int, float)):
        return float(encoded)
    if kind == "arrayValue" and isinstance(encoded, Mapping):
        values = encoded.get("values", [])
        if not isinstance(values, list):
            raise ValueError("OTLP array values must be a list")
        return [_decode_any_value(item) for item in values]
    raise ValueError(f"unsupported OTLP AnyValue field: {kind}")


def _encode_attributes(values: Mapping[str, object]) -> list[JsonObject]:
    return [
        {"key": key, "value": _encode_any_value(values[key])}
        for key in sorted(values)
    ]


def _decode_attributes(values: object) -> dict[str, object]:
    if values is None:
        return {}
    if not isinstance(values, list):
        raise ValueError("OTLP attributes must be a list")
    result: dict[str, object] = {}
    for item in values:
        if not isinstance(item, Mapping):
            raise ValueError("OTLP attribute must be an object")
        key = item.get("key")
        encoded = item.get("value")
        if not isinstance(key, str) or not isinstance(encoded, Mapping):
            raise ValueError("OTLP attribute requires a key and AnyValue")
        if key in result:
            raise ValueError(f"duplicate OTLP attribute: {key}")
        result[key] = _decode_any_value(encoded)
    return result


def _flatten(prefix: str, value: Mapping[str, object], out: dict[str, object]) -> None:
    for key in sorted(value):
        item = value[key]
        attribute_key = f"{prefix}.{key}"
        if isinstance(item, Mapping):
            _flatten(attribute_key, item, out)
        elif isinstance(item, (str, bool, int, float, list)):
            out[attribute_key] = item
        else:
            raise TypeError(f"unsupported canonical value at {attribute_key}")


def _assign_path(target: dict[str, object], path: tuple[str, ...], value: object) -> None:
    cursor = target
    for part in path[:-1]:
        existing = cursor.setdefault(part, {})
        if not isinstance(existing, dict):
            raise ValueError("flattened OTLP attributes contain a path collision")
        cursor = existing
    cursor[path[-1]] = value


def _event_name(record: Mapping[str, object]) -> str:
    kind = str(record["kind"])
    operation = str(record["operation"])
    if kind == "action":
        payload = record["payload"]
        assert isinstance(payload, Mapping)
        if payload.get("activation") == "primary" and payload.get("input") in {
            "touch",
            "pointer",
        }:
            return "app.widget.click"
        return "chill.action"
    if kind in {"event", "impression", "replay"}:
        return f"chill.{kind}"
    return f"chill.{kind}.{operation}"


def _derived_attributes(record: Mapping[str, object]) -> dict[str, object]:
    result: dict[str, object] = {"chill.otel.semconv.version": _SEMCONV_VERSION}
    context = record.get("context")
    if isinstance(context, Mapping):
        session_id = context.get("session_id")
        if isinstance(session_id, str):
            result["session.id"] = session_id
        page = context.get("page")
        if isinstance(page, Mapping):
            instance_id = page.get("instance_id")
            path = page.get("path")
            if isinstance(instance_id, str):
                result["app.screen.id"] = instance_id
            if isinstance(path, list) and path and isinstance(path[-1], str):
                result["app.screen.name"] = path[-1]
    if record.get("kind") == "action":
        payload = record["payload"]
        assert isinstance(payload, Mapping)
        element_id = payload.get("element_id")
        if isinstance(element_id, str):
            result["app.widget.id"] = element_id
    return result


def canonical_to_otlp_logs(record: Mapping[str, object]) -> JsonObject:
    """Encode one trusted canonical record as a lossless OTLP Logs request."""

    source = record.get("source")
    if not isinstance(source, Mapping):
        raise TypeError("canonical source must be an object")
    attributes: dict[str, object] = {
        "chill.schema.version": record["schema_version"],
        "chill.schema.url": record["schema_url"],
        "chill.record.id": record["record_id"],
        "chill.subject.id": record["subject_id"],
        "chill.behavior.kind": record["kind"],
        "chill.behavior.operation": record["operation"],
        "chill.behavior.name": record["name"],
    }
    for section in (
        "tenant",
        "source",
        "actor",
        "context",
        "privacy",
        "outcome",
        "payload",
    ):
        value = record.get(section)
        if isinstance(value, Mapping):
            if section == "privacy":
                classifications = value.get("annotation_classifications", {})
                if not isinstance(classifications, Mapping):
                    raise TypeError("annotation classifications must be an object")
                for key, classification in classifications.items():
                    attributes[f"{_ANNOTATION_CLASSIFICATION_PREFIX}{key}"] = classification
                value = {
                    key: item
                    for key, item in value.items()
                    if key != "annotation_classifications"
                }
            _flatten(f"chill.{section}", value, attributes)
    annotations = record.get("annotations", {})
    if not isinstance(annotations, Mapping):
        raise TypeError("canonical annotations must be an object")
    for key, value in annotations.items():
        attributes[f"{_ANNOTATION_PREFIX}{key}"] = value
    clock = record["clock"]
    if not isinstance(clock, Mapping):
        raise TypeError("canonical clock must be an object")
    for key in ("monotonic_nano", "boot_id", "sequence_number"):
        if key in clock:
            attributes[f"chill.clock.{key}"] = clock[key]
    if "duration_nano" in record:
        attributes["chill.duration_nano"] = record["duration_nano"]

    trace = record.get("trace")
    if isinstance(trace, Mapping):
        if "trace_state" in trace:
            trace_state = trace["trace_state"]
            if not isinstance(trace_state, str):
                raise TypeError("canonical trace_state must be a string")
            _validate_trace_state(trace_state)
        for key in ("parent_span_id", "trace_state"):
            if key in trace:
                attributes[f"chill.trace.{key}"] = trace[key]
    attributes.update(_derived_attributes(record))

    resource = record["resource"]
    if not isinstance(resource, Mapping):
        raise TypeError("canonical resource must be an object")
    resource_values = resource.get("attributes")
    if not isinstance(resource_values, Mapping):
        raise TypeError("canonical resource attributes must be an object")
    otel_resource = dict(resource_values)

    log_record: JsonObject = {
        "timeUnixNano": str(clock["occurred_at_unix_nano"]),
        "observedTimeUnixNano": str(clock["observed_at_unix_nano"]),
        "attributes": _encode_attributes(attributes),
        "eventName": _event_name(record),
    }
    if isinstance(trace, Mapping):
        log_record["traceId"] = trace["trace_id"]
        if "span_id" in trace:
            log_record["spanId"] = trace["span_id"]
        if "trace_flags" in trace:
            log_record["flags"] = int(str(trace["trace_flags"]), 16)
    payload = record["payload"]
    if isinstance(payload, Mapping):
        severity = payload.get("severity")
        if isinstance(severity, str):
            log_record["severityNumber"] = _SEVERITY_NUMBER[severity]

    instrumentation = record["instrumentation"]
    if not isinstance(instrumentation, Mapping):
        raise TypeError("canonical instrumentation must be an object")
    scope: JsonObject = {
        "name": instrumentation["name"],
        "version": instrumentation["version"],
    }
    scope_logs: JsonObject = {
        "scope": scope,
        "logRecords": [log_record],
    }
    if "schema_url" in instrumentation:
        scope_logs["schemaUrl"] = instrumentation["schema_url"]

    return {
        "resourceLogs": [
            {
                "resource": {"attributes": _encode_attributes(otel_resource)},
                "scopeLogs": [scope_logs],
            }
        ]
    }


def otlp_logs_to_canonical(request: Mapping[str, object]) -> JsonObject:
    """Decode the one-record reference encoding back into canonical JSON."""

    resource_logs = request.get("resourceLogs")
    if not isinstance(resource_logs, list) or len(resource_logs) != 1:
        raise ValueError("reference request must have exactly one resourceLogs item")
    resource_item = resource_logs[0]
    if not isinstance(resource_item, Mapping):
        raise ValueError("resourceLogs item must be an object")
    resource = resource_item.get("resource")
    scope_logs = resource_item.get("scopeLogs")
    if not isinstance(resource, Mapping):
        raise ValueError("OTLP resource must be an object")
    if not isinstance(scope_logs, list) or len(scope_logs) != 1:
        raise ValueError("reference request must have exactly one scopeLogs item")
    scope_item = scope_logs[0]
    if not isinstance(scope_item, Mapping):
        raise ValueError("scopeLogs item must be an object")
    scope = scope_item.get("scope")
    log_records = scope_item.get("logRecords")
    if not isinstance(scope, Mapping):
        raise ValueError("OTLP instrumentation scope must be an object")
    if not isinstance(log_records, list) or len(log_records) != 1:
        raise ValueError("reference request must have exactly one LogRecord")
    log = log_records[0]
    if not isinstance(log, Mapping):
        raise ValueError("OTLP LogRecord must be an object")

    attributes = _decode_attributes(log.get("attributes"))
    result: JsonObject = {
        "schema_version": attributes.pop("chill.schema.version"),
        "schema_url": attributes.pop("chill.schema.url"),
        "record_id": attributes.pop("chill.record.id"),
        "subject_id": attributes.pop("chill.subject.id"),
        "kind": attributes.pop("chill.behavior.kind"),
        "operation": attributes.pop("chill.behavior.operation"),
        "name": attributes.pop("chill.behavior.name"),
    }
    for key in _DERIVED_ATTRIBUTE_KEYS:
        attributes.pop(key, None)

    sections: dict[str, dict[str, object]] = {
        section: {}
        for section in (
            "tenant",
            "source",
            "actor",
            "context",
            "privacy",
            "outcome",
            "payload",
        )
    }
    annotations: dict[str, object] = {}
    annotation_classifications: dict[str, object] = {}
    clock: dict[str, object] = {
        "occurred_at_unix_nano": str(log["timeUnixNano"]),
        "observed_at_unix_nano": str(log["observedTimeUnixNano"]),
    }
    trace_extras: dict[str, object] = {}
    duration: object | None = None
    for key, value in attributes.items():
        if key.startswith(_ANNOTATION_CLASSIFICATION_PREFIX):
            annotation_classifications[
                key.removeprefix(_ANNOTATION_CLASSIFICATION_PREFIX)
            ] = value
            continue
        if key.startswith(_ANNOTATION_PREFIX):
            annotations[key.removeprefix(_ANNOTATION_PREFIX)] = value
            continue
        if key.startswith("chill.clock."):
            clock[key.removeprefix("chill.clock.")] = value
            continue
        if key.startswith("chill.trace."):
            trace_extras[key.removeprefix("chill.trace.")] = value
            continue
        if key == "chill.duration_nano":
            duration = value
            continue
        matched = False
        for section, target in sections.items():
            prefix = f"chill.{section}."
            if key.startswith(prefix):
                _assign_path(target, tuple(key.removeprefix(prefix).split(".")), value)
                matched = True
                break
        if not matched:
            raise ValueError(f"unknown Chill OTLP attribute: {key}")

    sections["privacy"]["annotation_classifications"] = annotation_classifications
    for section, value in sections.items():
        if value:
            result[section] = value
    result["clock"] = clock
    if "traceId" in log:
        trace: dict[str, object] = {
            "trace_id": log["traceId"],
            **trace_extras,
        }
        if "spanId" in log:
            trace["span_id"] = log["spanId"]
        if "flags" in log:
            trace["trace_flags"] = f"{int(log['flags']):02x}"
        result["trace"] = trace

    instrumentation: dict[str, object] = {
        "name": scope["name"],
        "version": scope["version"],
    }
    if "schemaUrl" in scope_item:
        instrumentation["schema_url"] = scope_item["schemaUrl"]
    result["instrumentation"] = instrumentation

    result["resource"] = {
        "attributes": _decode_attributes(resource.get("attributes"))
    }
    result["annotations"] = annotations
    if duration is not None:
        result["duration_nano"] = duration

    expected_event_name = _event_name(result)
    if log.get("eventName") != expected_event_name:
        raise ValueError("OTLP eventName does not match the Chill record shape")
    return result


def activity_records_to_otlp_span(
    start: Mapping[str, object], end: Mapping[str, object]
) -> JsonObject:
    """Project matching activity lifecycle facts into one OTLP internal span."""

    for record, operation in ((start, "start"), (end, "end")):
        if record.get("kind") != "activity" or record.get("operation") != operation:
            raise ValueError(f"expected an activity {operation} record")
    for key in (
        "subject_id",
        "name",
        "trace",
        "instrumentation",
        "resource",
        "source",
    ):
        if start.get(key) != end.get(key):
            raise ValueError(f"activity lifecycle mismatch: {key}")
    for key in ("context", "annotations", "payload"):
        if start.get(key) != end.get(key):
            raise ValueError(f"activity snapshot mismatch: {key}")

    trace = start["trace"]
    if not isinstance(trace, Mapping) or "span_id" not in trace:
        raise ValueError("activity span projection requires trace and span IDs")
    if "trace_state" in trace:
        trace_state = trace["trace_state"]
        if not isinstance(trace_state, str):
            raise TypeError("activity trace_state must be a string")
        _validate_trace_state(trace_state)
    start_clock = start["clock"]
    end_clock = end["clock"]
    payload = end["payload"]
    if not all(isinstance(item, Mapping) for item in (start_clock, end_clock, payload)):
        raise TypeError("activity clocks and payload must be objects")
    start_nano = int(str(start_clock["occurred_at_unix_nano"]))
    end_nano = int(str(end_clock["occurred_at_unix_nano"]))
    if end_nano < start_nano:
        raise ValueError("activity span ends before it starts")

    attributes: dict[str, object] = {
        "chill.activity.id": start["subject_id"],
        "chill.activity.start_record_id": start["record_id"],
        "chill.activity.end_record_id": end["record_id"],
        "chill.activity.kind": payload["activity_kind"],
        "chill.activity.role": payload["role"],
        "chill.activity.attempt": payload["attempt"],
        "chill.activity.recursion_depth": payload["recursion_depth"],
        "chill.duration_nano": end["duration_nano"],
    }
    source = start["source"]
    if not isinstance(source, Mapping):
        raise TypeError("activity source must be an object")
    _flatten("chill.source", source, attributes)
    context = end.get("context")
    if isinstance(context, Mapping):
        _flatten("chill.context", context, attributes)
    attributes.update(_derived_attributes(end))
    annotations = end.get("annotations", {})
    if isinstance(annotations, Mapping):
        for key, value in annotations.items():
            attributes[f"{_ANNOTATION_PREFIX}{key}"] = value
    outcome = end.get("outcome")
    if not isinstance(outcome, Mapping):
        raise ValueError("activity end requires an outcome")
    status = str(outcome["status"])
    attributes["chill.outcome.status"] = status
    if "reason_code" in outcome:
        attributes["chill.outcome.reason_code"] = outcome["reason_code"]
        if status in {"error", "timeout"}:
            attributes["error.type"] = outcome["reason_code"]

    otel_status = 1 if status == "ok" else 2 if status in {"error", "timeout"} else 0
    span: JsonObject = {
        "traceId": trace["trace_id"],
        "spanId": trace["span_id"],
        "name": start["name"],
        "kind": 1,
        "startTimeUnixNano": str(start_nano),
        "endTimeUnixNano": str(end_nano),
        "attributes": _encode_attributes(attributes),
        "flags": int(str(trace.get("trace_flags", "00")), 16),
        "status": {"code": otel_status},
    }
    if "parent_span_id" in trace:
        span["parentSpanId"] = trace["parent_span_id"]
    if "trace_state" in trace:
        span["traceState"] = trace["trace_state"]

    instrumentation = start["instrumentation"]
    resource = start["resource"]
    if not all(isinstance(item, Mapping) for item in (instrumentation, resource)):
        raise TypeError("activity resource and instrumentation must be objects")
    resource_values = resource.get("attributes")
    if not isinstance(resource_values, Mapping):
        raise TypeError("activity resource attributes must be an object")
    otel_resource = dict(resource_values)
    scope_spans: JsonObject = {
        "scope": {
            "name": instrumentation["name"],
            "version": instrumentation["version"],
        },
        "spans": [span],
    }
    if "schema_url" in instrumentation:
        scope_spans["schemaUrl"] = instrumentation["schema_url"]
    return {
        "resourceSpans": [
            {
                "resource": {"attributes": _encode_attributes(otel_resource)},
                "scopeSpans": [scope_spans],
            }
        ]
    }


def format_traceparent(context: SpanContext) -> str:
    return (
        f"00-{context.trace_id}-{context.span_id}-{context.trace_flags & 0x01:02x}"
    )


def parse_traceparent(value: str) -> SpanContext | None:
    """Parse W3C version 00; invalid input restarts rather than partially trusting."""

    parts = value.split("-")
    if len(parts) != 4 or parts[0] != "00":
        return None
    trace_id, span_id, flags = parts[1:]
    if not _TRACE_ID.fullmatch(trace_id) or not _SPAN_ID.fullmatch(span_id):
        return None
    if not re.fullmatch(r"[0-9a-f]{2}", flags):
        return None
    return SpanContext(trace_id, span_id, int(flags, 16), remote=True)


def filter_baggage(
    values: Mapping[str, str],
    policy: PropagationPolicy,
    *,
    trusted_source: bool = True,
) -> tuple[tuple[str, str], ...]:
    """Apply Chill's stricter allowlist and byte budgets before propagation."""

    if not trusted_source:
        return ()
    accepted: list[tuple[str, str]] = []
    encoded_size = 0
    for key in sorted(values):
        if key not in policy.baggage_allowlist or not _BAGGAGE_KEY.fullmatch(key):
            continue
        raw_value = values[key]
        if len(raw_value.encode("utf-8")) > policy.max_baggage_value_bytes:
            continue
        encoded = quote(
            raw_value,
            safe="!#$&'()*+-./:<=>?@[]^_`{|}~",
            encoding="utf-8",
            errors="strict",
        )
        entry_size = len(key.encode("ascii")) + 1 + len(encoded.encode("ascii"))
        separator_size = 1 if accepted else 0
        if len(accepted) >= policy.max_baggage_entries:
            break
        if encoded_size + separator_size + entry_size > policy.max_baggage_header_bytes:
            continue
        accepted.append((key, encoded))
        encoded_size += separator_size + entry_size
    return tuple(accepted)


def inject_http_headers(
    context: SpanContext,
    destination_origin: str,
    policy: PropagationPolicy,
    *,
    baggage: Mapping[str, str] | None = None,
) -> dict[str, str]:
    """Inject only across a configured trace trust boundary."""

    if destination_origin not in policy.trusted_origins:
        return {}
    headers = {"traceparent": format_traceparent(context)}
    if context.trace_state:
        headers["tracestate"] = context.trace_state
    if baggage:
        accepted = filter_baggage(baggage, policy)
        if accepted:
            headers["baggage"] = ",".join(f"{key}={value}" for key, value in accepted)
    return headers


def decide_messaging_context(
    ambient: SpanContext | None,
    message_creation_contexts: Iterable[SpanContext],
) -> MessagingDecision:
    """Use ambient parentage and message links, including for batches/fan-out."""

    unique: dict[tuple[str, str], SpanContext] = {}
    for context in message_creation_contexts:
        unique.setdefault((context.trace_id, context.span_id), context)
    return MessagingDecision(parent=ambient, links=tuple(unique.values()))
