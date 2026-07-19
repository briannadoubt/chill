#!/usr/bin/env python3
"""Validate Chill JSON Schemas and their example fixtures.

Requires the `jsonschema` package. The script deliberately has no knowledge of
the record fields so the checked-in schema remains the single validation source.
"""

from __future__ import annotations

from copy import deepcopy
import json
from pathlib import Path
import sys

try:
    from jsonschema import Draft202012Validator, FormatChecker
except ModuleNotFoundError:
    print(
        "error: install the jsonschema package to validate contract fixtures",
        file=sys.stderr,
    )
    raise SystemExit(2)


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_PATH = ROOT / "schemas/behavior/v1/envelope.schema.json"
EXAMPLES_PATH = ROOT / "examples/behavior/v1"


def reject_nonfinite(value: str) -> object:
    raise ValueError(f"non-finite JSON number is forbidden: {value}")


def load_json(path: Path) -> object:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle, parse_constant=reject_nonfinite)


def negative_cases(example: dict[str, object]) -> list[tuple[str, dict[str, object]]]:
    cases: list[tuple[str, dict[str, object]]] = []

    lifecycle_action = deepcopy(example)
    lifecycle_action["operation"] = "start"
    cases.append(("action rejects lifecycle operation", lifecycle_action))

    non_v7_record = deepcopy(example)
    non_v7_record["record_id"] = "8c735b90-6f71-4d73-9b16-6a42066f3d31"
    cases.append(("record rejects non-v7 UUID", non_v7_record))

    zero_trace = deepcopy(example)
    trace = zero_trace.setdefault("trace", {})
    assert isinstance(trace, dict)
    trace["trace_id"] = "0" * 32
    cases.append(("trace rejects all-zero ID", zero_trace))

    injectable_trace_state = deepcopy(example)
    trace = injectable_trace_state.setdefault("trace", {})
    assert isinstance(trace, dict)
    trace["trace_state"] = "vendor=header\r\ninjection"
    cases.append(("trace rejects injectable trace state", injectable_trace_state))

    nested_annotation = deepcopy(example)
    annotations = nested_annotation.setdefault("annotations", {})
    assert isinstance(annotations, dict)
    annotations["invalid.nested"] = {"secret": "value"}
    cases.append(("annotation rejects nested object", nested_annotation))

    unknown_field = deepcopy(example)
    unknown_field["unknown"] = True
    cases.append(("envelope rejects unknown field", unknown_field))

    misaligned_page_path = deepcopy(example)
    context = misaligned_page_path["context"]
    assert isinstance(context, dict)
    page = context["page"]
    assert isinstance(page, dict)
    path_instance_ids = page["path_instance_ids"]
    assert isinstance(path_instance_ids, list)
    path_instance_ids.pop(0)
    cases.append(("page rejects misaligned structured ancestry", misaligned_page_path))

    wrong_page_leaf = deepcopy(example)
    context = wrong_page_leaf["context"]
    assert isinstance(context, dict)
    page = context["page"]
    assert isinstance(page, dict)
    path_instance_ids = page["path_instance_ids"]
    assert isinstance(path_instance_ids, list)
    path_instance_ids[-1] = "0190f2a0-ffff-7000-8000-000000000099"
    cases.append(("page rejects ancestry ending at another instance", wrong_page_leaf))

    return cases


def semantic_negative_cases(
    activity: dict[str, object], event: dict[str, object]
) -> list[tuple[str, dict[str, object]]]:
    cases: list[tuple[str, dict[str, object]]] = []

    missing_activity_duration = deepcopy(activity)
    missing_activity_duration.pop("duration_nano")
    cases.append(("activity end requires duration", missing_activity_duration))

    invalid_operation_attempt = deepcopy(activity)
    payload = invalid_operation_attempt["payload"]
    assert isinstance(payload, dict)
    payload["attempt"] = 2
    cases.append(("operation activity rejects retry ordinal", invalid_operation_attempt))

    invalid_activity_update = deepcopy(activity)
    invalid_activity_update["operation"] = "update"
    cases.append(("activity rejects update operation", invalid_activity_update))

    failed_success_event = deepcopy(event)
    outcome = failed_success_event["outcome"]
    assert isinstance(outcome, dict)
    outcome["status"] = "error"
    cases.append(("succeeded event requires ok outcome", failed_success_event))

    entered_terminal_event = deepcopy(event)
    payload = entered_terminal_event["payload"]
    assert isinstance(payload, dict)
    payload["emission"] = "entered"
    cases.append(("entered event rejects terminal outcome", entered_terminal_event))

    return cases


def semantic_errors(record: dict[str, object]) -> list[str]:
    errors: list[str] = []
    kind = record["kind"]
    operation = record["operation"]
    record_id = record["record_id"]
    subject_id = record["subject_id"]
    clock = record["clock"]
    payload = record["payload"]
    context = record.get("context")

    assert isinstance(clock, dict)
    assert isinstance(payload, dict)
    assert context is None or isinstance(context, dict)

    observed = int(str(clock["observed_at_unix_nano"]))
    occurred = int(str(clock["occurred_at_unix_nano"]))
    if observed < occurred:
        errors.append("observed time precedes occurred time")

    if kind in {"action", "impression", "event"} and subject_id != record_id:
        errors.append(f"instant {kind} subject_id must equal record_id")

    if kind == "activity":
        if operation == "start" and (
            "outcome" in record or "duration_nano" in record
        ):
            errors.append("activity start cannot have outcome or duration")
        if operation == "end" and (
            "outcome" not in record or "duration_nano" not in record
        ):
            errors.append("activity end requires outcome and duration")
        if payload.get("role") == "operation" and payload.get("attempt") != 1:
            errors.append("an operation activity must have attempt 1")
        if payload.get("role") == "attempt" and not payload.get("parent_activity_id"):
            errors.append("an attempt activity requires parent_activity_id")
        if payload.get("parent_activity_id") == subject_id:
            errors.append("an activity cannot be its own parent")
        if payload.get("parent_activity_id") is not None and (
            context is None
            or context.get("activity_id") != payload.get("parent_activity_id")
        ):
            errors.append("activity context must identify its parent activity")

    if kind == "event":
        emission = payload.get("emission")
        outcome = record.get("outcome")
        assert outcome is None or isinstance(outcome, dict)
        if emission == "entered" and outcome is not None:
            errors.append("an entered event cannot claim a terminal outcome")
        if emission == "succeeded" and (
            outcome is None or outcome.get("status") != "ok"
        ):
            errors.append("a succeeded event requires an ok outcome")
        if emission == "terminal" and outcome is None:
            errors.append("a terminal event requires an outcome")

    if kind == "session" and context is not None and subject_id != context.get("session_id"):
        errors.append("session subject_id must equal context.session_id")

    if kind == "journey" and context is not None and subject_id != context.get("journey_id"):
        errors.append("journey subject_id must equal context.journey_id")

    if kind == "page":
        if subject_id != payload.get("instance_id"):
            errors.append("page subject_id must equal payload.instance_id")
        if context is not None:
            page = context.get("page")
            assert page is None or isinstance(page, dict)
            if page is not None and (
                page.get("surface_id") != payload.get("surface_id")
                or page.get("instance_id") != payload.get("instance_id")
                or page.get("path") != payload.get("path")
                or page.get("path_instance_ids")
                != payload.get("path_instance_ids")
            ):
                errors.append("page context must match page payload")

    if context is not None:
        page = context.get("page")
        assert page is None or isinstance(page, dict)
        if page is not None:
            path = page.get("path")
            path_instance_ids = page.get("path_instance_ids")
            assert isinstance(path, list)
            assert isinstance(path_instance_ids, list)
            if len(path) != len(path_instance_ids):
                errors.append("page path and instance ancestry must have equal length")
            if path_instance_ids and path_instance_ids[-1] != page.get("instance_id"):
                errors.append("page path instance ancestry must end at page instance")

    if kind == "page":
        path = payload.get("path")
        path_instance_ids = payload.get("path_instance_ids")
        assert isinstance(path, list)
        assert isinstance(path_instance_ids, list)
        if len(path) != len(path_instance_ids):
            errors.append("page payload path and instance ancestry must have equal length")
        if path_instance_ids and path_instance_ids[-1] != payload.get("instance_id"):
            errors.append("page payload instance ancestry must end at page instance")
        if payload.get("focused") and payload.get("exposure") != "foreground":
            errors.append("a focused page must be foreground")

    if kind == "replay":
        if context is None or subject_id != context.get("replay_id"):
            errors.append("replay subject_id must equal context.replay_id")
        if int(str(payload["ends_at_unix_nano"])) < int(str(payload["starts_at_unix_nano"])):
            errors.append("replay chunk ends before it starts")

    if "duration_nano" in record and operation != "end":
        errors.append("duration_nano is only valid on end records")

    return errors


def main() -> int:
    schema = load_json(SCHEMA_PATH)
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema, format_checker=FormatChecker())

    failures = 0
    examples = sorted(EXAMPLES_PATH.glob("*.json"))
    if not examples:
        print(f"error: no fixtures found under {EXAMPLES_PATH}", file=sys.stderr)
        return 1

    for example in examples:
        record = load_json(example)
        errors = sorted(validator.iter_errors(record), key=lambda error: list(error.path))
        assert isinstance(record, dict)
        invariant_errors = semantic_errors(record)
        if not errors and not invariant_errors:
            print(f"ok: {example.relative_to(ROOT)}")
            continue

        failures += 1
        for error in errors:
            location = ".".join(str(part) for part in error.absolute_path) or "<root>"
            print(f"error: {example.relative_to(ROOT)}:{location}: {error.message}")
        for error in invariant_errors:
            print(f"error: {example.relative_to(ROOT)}:<semantic>: {error}")

    valid_action = load_json(EXAMPLES_PATH / "action-activated.json")
    assert isinstance(valid_action, dict)
    for case_name, invalid_record in negative_cases(valid_action):
        schema_valid = validator.is_valid(invalid_record)
        invariant_valid = not semantic_errors(invalid_record)
        if schema_valid and invariant_valid:
            failures += 1
            print(f"error: invalid case unexpectedly passed: {case_name}")
        else:
            print(f"ok: invalid case rejected: {case_name}")

    valid_activity = load_json(EXAMPLES_PATH / "activity-ended.json")
    valid_event = load_json(EXAMPLES_PATH / "domain-event.json")
    assert isinstance(valid_activity, dict)
    assert isinstance(valid_event, dict)
    for case_name, invalid_record in semantic_negative_cases(
        valid_activity, valid_event
    ):
        if validator.is_valid(invalid_record) and not semantic_errors(invalid_record):
            failures += 1
            print(f"error: invalid case unexpectedly passed: {case_name}")
        else:
            print(f"ok: invalid case rejected: {case_name}")

    if failures:
        print(f"failed: {failures} fixture(s)", file=sys.stderr)
        return 1

    print(f"validated: {len(examples)} fixture(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
