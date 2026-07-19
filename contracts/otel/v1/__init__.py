"""Version 1 OpenTelemetry mapping and propagation reference."""

from .reference import (
    MessagingDecision,
    PropagationPolicy,
    SpanContext,
    activity_records_to_otlp_span,
    canonical_to_otlp_logs,
    decide_messaging_context,
    filter_baggage,
    format_traceparent,
    inject_http_headers,
    otlp_logs_to_canonical,
    parse_traceparent,
)

__all__ = [
    "MessagingDecision",
    "PropagationPolicy",
    "SpanContext",
    "activity_records_to_otlp_span",
    "canonical_to_otlp_logs",
    "decide_messaging_context",
    "filter_baggage",
    "format_traceparent",
    "inject_http_headers",
    "otlp_logs_to_canonical",
    "parse_traceparent",
]
