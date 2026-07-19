from __future__ import annotations

import json
from pathlib import Path
import unittest

from contracts.otel.v1 import (
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


ROOT = Path(__file__).resolve().parents[2]
BEHAVIOR_EXAMPLES = ROOT / "examples" / "behavior" / "v1"
OTLP_EXAMPLES = ROOT / "examples" / "otlp" / "v1"


def fixture(name: str) -> dict[str, object]:
    with (BEHAVIOR_EXAMPLES / name).open(encoding="utf-8") as handle:
        value = json.load(handle)
    assert isinstance(value, dict)
    return value


def otlp_fixture(name: str) -> dict[str, object]:
    with (OTLP_EXAMPLES / name).open(encoding="utf-8") as handle:
        value = json.load(handle)
    assert isinstance(value, dict)
    return value


class OtlpRoundTripTests(unittest.TestCase):
    def test_action_log_matches_checked_otlp_wire_fixture(self) -> None:
        encoded = canonical_to_otlp_logs(fixture("action-activated.json"))

        self.assertEqual(encoded, otlp_fixture("action-log.json"))
        self.assertEqual(
            otlp_logs_to_canonical(otlp_fixture("action-log.json")),
            fixture("action-activated.json"),
        )

    def test_every_canonical_fixture_round_trips_through_otlp_logs(self) -> None:
        for path in sorted(BEHAVIOR_EXAMPLES.glob("*.json")):
            with self.subTest(path=path.name):
                record = fixture(path.name)
                encoded = canonical_to_otlp_logs(record)
                decoded = otlp_logs_to_canonical(encoded)
                self.assertEqual(decoded, record)

    def test_otlp_json_uses_hex_ids_integer_enums_and_decimal_int64(self) -> None:
        request = canonical_to_otlp_logs(fixture("action-activated.json"))
        resource_log = request["resourceLogs"][0]
        scope_log = resource_log["scopeLogs"][0]
        log = scope_log["logRecords"][0]

        self.assertEqual(log["traceId"], "4bf92f3577b34da6a3ce929d0e0e4736")
        self.assertEqual(log["spanId"], "00f067aa0ba902b7")
        self.assertEqual(log["flags"], 1)
        self.assertIsInstance(log["timeUnixNano"], str)
        self.assertEqual(log["eventName"], "app.widget.click")

    def test_standard_context_attributes_accompany_lossless_chill_fields(self) -> None:
        request = canonical_to_otlp_logs(fixture("action-activated.json"))
        log = request["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]
        attributes = {
            item["key"]: item["value"] for item in log["attributes"]
        }

        self.assertEqual(
            attributes["session.id"]["stringValue"],
            "0190f29f-ff00-7000-8000-000000000001",
        )
        self.assertEqual(
            attributes["app.widget.id"]["stringValue"], "adopt_button"
        )
        self.assertEqual(
            attributes["chill.record.id"]["stringValue"],
            "0190f2a1-2b3c-7d4e-8f50-1234567890ab",
        )

    def test_source_is_record_context_and_cannot_collide_with_resource(self) -> None:
        record = fixture("action-activated.json")
        resource = record["resource"]
        assert isinstance(resource, dict)
        resource_attributes = resource["attributes"]
        assert isinstance(resource_attributes, dict)
        resource_attributes["chill.source.platform"] = "resource-owned"

        request = canonical_to_otlp_logs(record)
        resource_items = request["resourceLogs"][0]["resource"]["attributes"]
        log_items = request["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0][
            "attributes"
        ]
        encoded_resource = {item["key"]: item["value"] for item in resource_items}
        encoded_log = {item["key"]: item["value"] for item in log_items}

        self.assertEqual(
            encoded_resource["chill.source.platform"]["stringValue"],
            "resource-owned",
        )
        self.assertEqual(
            encoded_log["chill.source.platform"]["stringValue"], "apple"
        )
        self.assertEqual(otlp_logs_to_canonical(request), record)

    def test_maximum_annotation_set_is_not_silently_truncated(self) -> None:
        record = fixture("action-activated.json")
        record["annotations"] = {
            f"test.key_{index:03d}": index for index in range(128)
        }
        privacy = record["privacy"]
        assert isinstance(privacy, dict)
        privacy["annotation_classifications"] = {
            key: "internal" for key in record["annotations"]
        }

        request = canonical_to_otlp_logs(record)
        log = request["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]

        self.assertGreater(len(log["attributes"]), 128)
        self.assertNotIn("droppedAttributesCount", log)
        self.assertEqual(otlp_logs_to_canonical(request), record)

    def test_activity_lifecycle_projects_to_one_internal_span(self) -> None:
        request = activity_records_to_otlp_span(
            fixture("activity-started.json"), fixture("activity-ended.json")
        )
        self.assertEqual(request, otlp_fixture("activity-span.json"))
        span = request["resourceSpans"][0]["scopeSpans"][0]["spans"][0]
        attributes = {
            item["key"]: item["value"] for item in span["attributes"]
        }

        self.assertEqual(span["traceId"], "4bf92f3577b34da6a3ce929d0e0e4736")
        self.assertEqual(span["spanId"], "a3ce929d0e0e4736")
        self.assertEqual(span["parentSpanId"], "00f067aa0ba902b7")
        self.assertEqual(span["kind"], 1)
        self.assertEqual(span["status"], {"code": 1})
        self.assertEqual(span["name"], "adoption.submit")
        self.assertEqual(
            attributes["chill.activity.id"]["stringValue"],
            "0190f2a1-3000-7000-8000-000000000005",
        )

    def test_mismatched_activity_snapshots_do_not_form_a_span(self) -> None:
        start = fixture("activity-started.json")
        end = fixture("activity-ended.json")
        end["annotations"] = {"cat.id": "different"}

        with self.assertRaisesRegex(ValueError, "snapshot mismatch"):
            activity_records_to_otlp_span(start, end)


class PropagationTests(unittest.TestCase):
    CONTEXT = SpanContext(
        "4bf92f3577b34da6a3ce929d0e0e4736",
        "00f067aa0ba902b7",
        1,
        "vendor=value",
    )

    def policy(self) -> PropagationPolicy:
        return PropagationPolicy(
            trusted_origins=frozenset({"https://api.example.test"}),
            baggage_allowlist=frozenset({"release.channel"}),
        )

    def test_traceparent_round_trip_and_invalid_context_rejection(self) -> None:
        encoded = format_traceparent(self.CONTEXT)
        self.assertEqual(
            encoded,
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )
        self.assertEqual(
            parse_traceparent(encoded),
            SpanContext(
                self.CONTEXT.trace_id,
                self.CONTEXT.span_id,
                1,
                remote=True,
            ),
        )
        self.assertIsNone(
            parse_traceparent("00-00000000000000000000000000000000-00f067aa0ba902b7-01")
        )

    def test_tracestate_rejects_noncanonical_or_injectable_values(self) -> None:
        for invalid in (
            "UPPER=value",
            "vendor=value,vendor=duplicate",
            "vendor=trailing ",
            "vendor=header\r\ninjection",
            "vendor=value=extra",
        ):
            with self.subTest(invalid=invalid):
                with self.assertRaises(ValueError):
                    SpanContext(
                        self.CONTEXT.trace_id,
                        self.CONTEXT.span_id,
                        trace_state=invalid,
                    )

        record = fixture("action-activated.json")
        trace = record["trace"]
        assert isinstance(trace, dict)
        trace["trace_state"] = "vendor=header\r\ninjection"
        with self.assertRaises(ValueError):
            canonical_to_otlp_logs(record)

    def test_http_injection_requires_a_trusted_origin(self) -> None:
        trusted = inject_http_headers(
            self.CONTEXT,
            "https://api.example.test",
            self.policy(),
            baggage={
                "release.channel": "internal beta",
                "session.id": "must-not-leak",
            },
        )
        untrusted = inject_http_headers(
            self.CONTEXT,
            "https://third-party.example",
            self.policy(),
        )

        self.assertIn("traceparent", trusted)
        self.assertEqual(trusted["tracestate"], "vendor=value")
        self.assertEqual(trusted["baggage"], "release.channel=internal%20beta")
        self.assertEqual(untrusted, {})

    def test_baggage_is_allowlisted_bounded_and_rejects_untrusted_sources(self) -> None:
        policy = PropagationPolicy(
            trusted_origins=frozenset(),
            baggage_allowlist=frozenset({"allowed", "too-long"}),
            max_baggage_entries=1,
            max_baggage_value_bytes=8,
        )

        self.assertEqual(
            filter_baggage(
                {"allowed": "ok", "too-long": "0123456789", "user.id": "secret"},
                policy,
            ),
            (("allowed", "ok"),),
        )
        self.assertEqual(
            filter_baggage({"allowed": "ok"}, policy, trusted_source=False), ()
        )

    def test_messaging_uses_ambient_parent_and_message_creation_links(self) -> None:
        ambient = self.CONTEXT
        creation_one = SpanContext(
            "11111111111111111111111111111111", "1111111111111111"
        )
        creation_two = SpanContext(
            "22222222222222222222222222222222", "2222222222222222"
        )
        decision = decide_messaging_context(
            ambient, (creation_one, creation_two, creation_one)
        )

        self.assertEqual(decision.parent, ambient)
        self.assertEqual(decision.links, (creation_one, creation_two))


if __name__ == "__main__":
    unittest.main()
