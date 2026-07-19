"""Executable, platform-neutral Chill SDK conformance protocol.

The fixtures deliberately inject clocks, identifiers, salts, and failures.
Those controls belong only in test adapters; production SDK APIs remain
declarative and automatic.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass
import hashlib
import json
from pathlib import Path
import re
from typing import Any, Callable, Mapping, Sequence

from contracts.annotations.v1 import (
    AnnotationDeclaration,
    AnnotationScope,
    resolve_annotations,
)
from contracts.instrumentation.v1 import (
    InputKind,
    NativeActivation,
    RuntimeState,
    observe_action,
)
from contracts.navigation.v1 import (
    NavigationState,
    Relation,
    RouteSpec,
    dismiss,
    present,
    reconcile_linear_path,
)
from contracts.otel.v1 import (
    PropagationPolicy,
    SpanContext,
    decide_messaging_context,
    inject_http_headers,
    parse_traceparent,
)


JsonObject = dict[str, Any]
_SUITE_VERSION = "1.0.0"
_SEMVER = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
_DOMAINS = {
    "annotations",
    "navigation",
    "actions",
    "trace",
    "offline",
    "replay",
    "schema",
    "redaction",
    "sampling",
}
_LAYERS = {"model", "adapter", "wire"}
_PROFILE_DOMAINS = {
    "client.core": _DOMAINS - {"replay"},
    "client.replay": _DOMAINS,
    "server.core": {
        "annotations",
        "trace",
        "offline",
        "schema",
        "redaction",
        "sampling",
    },
}


class ConformanceSuiteError(ValueError):
    """The checked suite is malformed or incomplete."""


class ConformanceReportError(ValueError):
    """An SDK implementation report is malformed or incomplete."""


@dataclass(frozen=True)
class ScenarioResult:
    scenario_id: str
    domain: str
    layer: str
    passed: bool
    expected_digest: str
    actual_digest: str
    reasons: tuple[str, ...]
    actual: JsonObject

    def as_dict(self) -> JsonObject:
        return asdict(self)


@dataclass(frozen=True)
class SuiteEvaluation:
    suite_version: str
    suite_digest: str
    passed: bool
    results: tuple[ScenarioResult, ...]

    def as_dict(self) -> JsonObject:
        return asdict(self)


@dataclass(frozen=True)
class ImplementationEvaluation:
    suite_version: str
    suite_digest: str
    profile: str
    implementation: JsonObject
    passed: bool
    results: tuple[ScenarioResult, ...]

    def as_dict(self) -> JsonObject:
        return asdict(self)


def _canonical_bytes(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def _digest(value: object) -> str:
    return hashlib.sha256(_canonical_bytes(value)).hexdigest()


def _require_object(value: object, label: str) -> JsonObject:
    if not isinstance(value, dict) or not all(
        isinstance(key, str) for key in value
    ):
        raise ConformanceSuiteError(f"{label} must be a JSON object")
    return value


def _load_json(path: Path) -> JsonObject:
    try:
        with path.open(encoding="utf-8") as handle:
            return _require_object(json.load(handle), str(path))
    except (OSError, json.JSONDecodeError) as error:
        raise ConformanceSuiteError(f"cannot load {path}: {error}") from error


def _resolve_profile_scenarios(
    manifest: Mapping[str, object], profile_id: str
) -> tuple[str, ...]:
    profiles = _require_object(manifest.get("profiles"), "profiles")
    seen: set[str] = set()

    def visit(current_id: str) -> list[str]:
        if current_id in seen:
            raise ConformanceSuiteError("profile inheritance contains a cycle")
        seen.add(current_id)
        profile = _require_object(profiles.get(current_id), f"profile {current_id}")
        result: list[str] = []
        parent = profile.get("extends")
        if parent is not None:
            if not isinstance(parent, str) or parent not in profiles:
                raise ConformanceSuiteError(f"profile {current_id} has unknown parent")
            result.extend(visit(parent))
        scenario_ids = profile.get("scenarios", [])
        if not isinstance(scenario_ids, list) or not all(
            isinstance(item, str) for item in scenario_ids
        ):
            raise ConformanceSuiteError(
                f"profile {current_id} scenarios must be string IDs"
            )
        result.extend(scenario_ids)
        seen.remove(current_id)
        return result

    resolved = visit(profile_id)
    if len(resolved) != len(set(resolved)):
        raise ConformanceSuiteError(f"profile {profile_id} repeats a scenario")
    return tuple(resolved)


def load_suite(manifest_path: Path) -> tuple[JsonObject, tuple[JsonObject, ...]]:
    manifest_path = manifest_path.resolve()
    manifest = _load_json(manifest_path)
    if manifest.get("suite_version") != _SUITE_VERSION:
        raise ConformanceSuiteError(f"suite version must be {_SUITE_VERSION}")

    required_domains = manifest.get("required_domains")
    if (
        not isinstance(required_domains, list)
        or not all(isinstance(item, str) for item in required_domains)
        or set(required_domains) != _DOMAINS
    ):
        raise ConformanceSuiteError(
            "required_domains must name every V1 domain exactly"
        )
    if len(required_domains) != len(set(required_domains)):
        raise ConformanceSuiteError("required_domains contains duplicates")

    descriptors = manifest.get("scenarios")
    if not isinstance(descriptors, list) or not descriptors:
        raise ConformanceSuiteError("manifest scenarios must be a nonempty list")

    root = manifest_path.parent
    scenarios: list[JsonObject] = []
    descriptor_ids: set[str] = set()
    for raw_descriptor in descriptors:
        descriptor = _require_object(raw_descriptor, "scenario descriptor")
        scenario_id = descriptor.get("id")
        domain = descriptor.get("domain")
        layers = descriptor.get("layers")
        relative_path = descriptor.get("path")
        if not all(
            isinstance(item, str) and item
            for item in (scenario_id, domain, relative_path)
        ):
            raise ConformanceSuiteError("scenario descriptor fields must be strings")
        if scenario_id in descriptor_ids:
            raise ConformanceSuiteError(f"duplicate scenario descriptor: {scenario_id}")
        if (
            domain not in _DOMAINS
            or not isinstance(layers, list)
            or not all(isinstance(item, str) for item in layers)
            or set(layers) != _LAYERS
            or len(layers) != len(set(layers))
        ):
            raise ConformanceSuiteError(f"invalid domain or layers for {scenario_id}")
        scenario_path = (root / relative_path).resolve()
        if not scenario_path.is_relative_to(root):
            raise ConformanceSuiteError("scenario path escapes the suite directory")
        scenario = _load_json(scenario_path)
        if (
            scenario.get("suite_version") != _SUITE_VERSION
            or scenario.get("id") != scenario_id
            or scenario.get("domain") != domain
            or scenario.get("layers") != layers
        ):
            raise ConformanceSuiteError(
                f"scenario {scenario_id} does not match its descriptor"
            )
        _require_object(scenario.get("given"), f"{scenario_id} given")
        _require_object(scenario.get("expect"), f"{scenario_id} expect")
        forbidden = scenario.get("forbidden_tokens", [])
        if not isinstance(forbidden, list) or not all(
            isinstance(token, str) and token for token in forbidden
        ):
            raise ConformanceSuiteError(
                f"{scenario_id} forbidden_tokens must be nonempty strings"
            )
        descriptor_ids.add(scenario_id)
        scenarios.append(scenario)

    if {scenario["domain"] for scenario in scenarios} != _DOMAINS:
        raise ConformanceSuiteError("scenario fixtures do not cover every V1 domain")

    profiles = _require_object(manifest.get("profiles"), "profiles")
    if set(profiles) != {"client.core", "client.replay", "server.core"}:
        raise ConformanceSuiteError("manifest must define the three V1 profiles")
    for profile_id in profiles:
        resolved = _resolve_profile_scenarios(manifest, profile_id)
        unknown = set(resolved) - descriptor_ids
        if unknown:
            raise ConformanceSuiteError(
                f"profile {profile_id} names unknown scenarios: {sorted(unknown)}"
            )
        domains = {
            scenario["domain"]
            for scenario in scenarios
            if scenario["id"] in resolved
        }
        if domains != _PROFILE_DOMAINS[profile_id]:
            raise ConformanceSuiteError(
                f"profile {profile_id} does not cover its required domains"
            )
    return manifest, tuple(scenarios)


def suite_digest(
    manifest: Mapping[str, object], scenarios: Sequence[Mapping[str, object]]
) -> str:
    """Bind a report to the exact manifest, inputs, expected outputs, and canaries."""

    return _digest({"manifest": manifest, "scenarios": list(scenarios)})


def _annotation_scenario(given: Mapping[str, object]) -> JsonObject:
    scopes: list[AnnotationScope] = []
    for raw_scope in given["scopes"]:
        declarations = tuple(
            AnnotationDeclaration(item["key"], item["value"])
            for item in raw_scope["declarations"]
        )
        scopes.append(
            AnnotationScope(raw_scope["scope_id"], raw_scope["depth"], declarations)
        )
    resolved = resolve_annotations(tuple(scopes))
    return {
        "values": {
            key: list(value) if isinstance(value, tuple) else value
            for key, value in resolved.values.items()
        },
        "origins": {
            key: asdict(origin) for key, origin in resolved.origins.items()
        },
        "collisions": [asdict(collision) for collision in resolved.collisions],
    }


def _path_json(path: object) -> JsonObject:
    return {
        "surface_id": path.surface_id,
        "instance_id": path.instance_id,
        "instance_ids": list(path.instance_ids),
        "segments": list(path.segments),
        "relation": path.relation.value,
        "exposure": path.exposure.value,
        "focused": path.focused,
    }


def _transition_json(operation: str, transition: object) -> JsonObject:
    return {
        "operation": operation,
        "delta": {
            "started": list(transition.delta.started),
            "updated": list(transition.delta.updated),
            "ended": list(transition.delta.ended),
        },
        "exposed_paths": [
            _path_json(path) for path in transition.state.exposed_paths()
        ],
    }


def _navigation_scenario(given: Mapping[str, object]) -> JsonObject:
    ids = iter(given["instance_ids"])
    state = NavigationState()
    transitions: list[JsonObject] = []
    desired = tuple(
        RouteSpec(item["segment"], item["identity"], Relation(item["relation"]))
        for item in given["linear_path"]
    )
    reconciled = reconcile_linear_path(
        state, given["surface_id"], desired, lambda: next(ids)
    )
    state = reconciled.state
    transitions.append(_transition_json("reconcile", reconciled))

    presentation = given["presentation"]
    presented = present(
        state,
        presentation["presenter_instance_id"],
        RouteSpec(
            presentation["segment"],
            presentation["identity"],
            Relation(presentation["relation"]),
        ),
        next(ids),
    )
    state = presented.state
    transitions.append(_transition_json("present", presented))
    dismissed = dismiss(state, presentation["instance_id"])
    state = dismissed.state
    transitions.append(_transition_json("dismiss", dismissed))
    return {
        "transitions": transitions,
        "primary_paths": [_path_json(path) for path in state.primary_paths()],
    }


def _action_fact_json(fact: object) -> JsonObject:
    return {
        "record_id": fact.record_id,
        "name": fact.name,
        "role": fact.role,
        "activation": fact.activation,
        "input_kind": fact.input_kind.value,
        "surface_id": fact.surface_id,
        "element_instance_id": fact.element_instance_id,
    }


def _actions_scenario(given: Mapping[str, object]) -> JsonObject:
    state = RuntimeState()
    facts: list[JsonObject] = []
    diagnostics: list[JsonObject] = []
    for index, item in enumerate(given["observations"]):
        transition = observe_action(
            state,
            NativeActivation(
                item["native_activation_id"],
                item["record_id"],
                item["surface_id"],
                item["element_instance_id"],
                item["name"],
                item["role"],
                item["activation"],
                InputKind(item["input_kind"]),
            ),
        )
        state = transition.state
        facts.extend(_action_fact_json(fact) for fact in transition.facts)
        diagnostics.extend(
            {"observation_index": index, "code": code}
            for code in transition.diagnostics
        )
    return {
        "facts": facts,
        "diagnostics": diagnostics,
        "deduplication_keys": [list(key) for key in state.seen_activation_ids],
    }


def _context_json(context: SpanContext | None) -> JsonObject | None:
    if context is None:
        return None
    return {
        "trace_id": context.trace_id,
        "span_id": context.span_id,
        "trace_flags": context.trace_flags,
        "trace_state": context.trace_state,
        "remote": context.remote,
    }


def _span_context(value: Mapping[str, object]) -> SpanContext:
    return SpanContext(
        str(value["trace_id"]),
        str(value["span_id"]),
        int(value.get("trace_flags", 0)),
        value.get("trace_state"),
        bool(value.get("remote", False)),
    )


def _trace_scenario(given: Mapping[str, object]) -> JsonObject:
    context = _span_context(given["context"])
    policy_value = given["policy"]
    policy = PropagationPolicy(
        frozenset(policy_value["trusted_origins"]),
        frozenset(policy_value["baggage_allowlist"]),
    )
    headers = {
        origin: inject_http_headers(
            context, origin, policy, baggage=given["baggage"]
        )
        for origin in given["destinations"]
    }
    messaging = given["messaging"]
    decision = decide_messaging_context(
        _span_context(messaging["ambient"]),
        [_span_context(item) for item in messaging["creation_contexts"]],
    )
    return {
        "http_headers": headers,
        "parsed_valid": _context_json(parse_traceparent(given["valid_traceparent"])),
        "parsed_invalid": _context_json(
            parse_traceparent(given["invalid_traceparent"])
        ),
        "messaging": {
            "parent": _context_json(decision.parent),
            "links": [_context_json(link) for link in decision.links],
        },
    }


def _offline_scenario(given: Mapping[str, object]) -> JsonObject:
    capacity = int(given["capacity_bytes"])
    priorities = {"replay": 0, "low": 1, "high": 2}
    queue: list[JsonObject] = []
    dropped: list[JsonObject] = []
    attempts: list[list[str]] = []
    recoveries = 0
    sequence = 0

    for step in given["steps"]:
        operation = step["operation"]
        if operation == "enqueue":
            sequence += 1
            queue.append(
                {
                    "record_id": step["record_id"],
                    "kind": step["kind"],
                    "priority": step["priority"],
                    "bytes": step["bytes"],
                    "sequence": sequence,
                }
            )
            while sum(item["bytes"] for item in queue) > capacity:
                victim = min(
                    queue,
                    key=lambda item: (priorities[item["priority"]], item["sequence"]),
                )
                queue.remove(victim)
                dropped.append(
                    {
                        "record_id": victim["record_id"],
                        "kind": victim["kind"],
                        "reason": "capacity",
                    }
                )
        elif operation == "crash_recover":
            recoveries += 1
        elif operation == "export":
            batch = queue[: int(step["max_records"])]
            attempts.append([item["record_id"] for item in batch])
            if step["result"] == "ack":
                acknowledged = set(step["acknowledged_record_ids"])
                batch_ids = {item["record_id"] for item in batch}
                if not acknowledged <= batch_ids:
                    raise ConformanceSuiteError(
                        "offline acknowledgement is not part of the attempted batch"
                    )
                queue = [
                    item
                    for item in queue
                    if item["record_id"] not in acknowledged
                ]
            elif step["result"] != "failure":
                raise ConformanceSuiteError(
                    "offline export result must be ack or failure"
                )
        else:
            raise ConformanceSuiteError(f"unknown offline operation: {operation}")
    return {
        "queued": [
            {
                "record_id": item["record_id"],
                "kind": item["kind"],
                "priority": item["priority"],
                "bytes": item["bytes"],
            }
            for item in queue
        ],
        "dropped": dropped,
        "export_attempts": attempts,
        "recovery_count": recoveries,
        "queued_bytes": sum(item["bytes"] for item in queue),
    }


def _replay_scenario(given: Mapping[str, object]) -> JsonObject:
    chunks = given["chunks"]
    aligned: list[JsonObject] = []
    for fact in given["facts"]:
        candidates = [
            chunk
            for chunk in chunks
            if chunk["boot_id"] == fact["boot_id"]
            and chunk["start_monotonic_nano"] <= fact["monotonic_nano"]
            < chunk["end_monotonic_nano"]
        ]
        if len(candidates) != 1:
            raise ConformanceSuiteError("replay fact must align with exactly one chunk")
        chunk = candidates[0]
        aligned.append(
            {
                "record_id": fact["record_id"],
                "chunk_id": chunk["chunk_id"],
                "offset_nano": fact["monotonic_nano"]
                - chunk["start_monotonic_nano"],
            }
        )
    return {"aligned_facts": aligned, "clock_basis": "boot_monotonic"}


def _schema_scenario(given: Mapping[str, object]) -> JsonObject:
    supported = given["supported"]
    results: list[JsonObject] = []
    for request in given["requests"]:
        version = request["schema_version"]
        url = request["schema_url"]
        if not isinstance(version, str) or not _SEMVER.fullmatch(version):
            accepted, reason = False, "invalid_version"
        elif version not in supported:
            major = int(version.split(".", 1)[0])
            supported_majors = {int(item.split(".", 1)[0]) for item in supported}
            reason = (
                "unsupported_version"
                if major in supported_majors
                else "unsupported_major"
            )
            accepted = False
        elif supported[version] != url:
            accepted, reason = False, "schema_url_mismatch"
        else:
            accepted, reason = True, "accepted_exact"
        results.append(
            {
                "case": request["case"],
                "accepted": accepted,
                "reason": reason,
                "negotiated_version": version if accepted else None,
            }
        )
    return {"results": results}


def _length_bucket(length: int) -> str:
    if length == 0:
        return "0"
    if length <= 4:
        return "1-4"
    if length <= 16:
        return "5-16"
    if length <= 64:
        return "17-64"
    return "65+"


def _redaction_scenario(given: Mapping[str, object]) -> JsonObject:
    output: list[JsonObject] = []
    for item in given["inputs"]:
        kind = item["kind"]
        if kind == "ui":
            redacted: JsonObject = {
                "kind": "ui",
                "element_id": item["element_id"],
                "role": item["role"],
                "geometry": item["geometry"],
            }
            if item.get("secure", False):
                redacted["content_mask"] = {"kind": "secure_input"}
            elif "text" in item:
                redacted["content_mask"] = {
                    "kind": "text",
                    "length_bucket": _length_bucket(len(item["text"])),
                }
            elif item.get("pixels", False):
                redacted["content_mask"] = {"kind": "pixels"}
            output.append(redacted)
        elif kind == "http":
            output.append(
                {
                    "kind": "http",
                    "method": item["method"],
                    "route_template": item["route_template"],
                    "status_code": item["status_code"],
                }
            )
        elif kind == "error":
            output.append({"kind": "error", "reason_code": item["reason_code"]})
        else:
            raise ConformanceSuiteError(f"unknown redaction input kind: {kind}")
    return {"records": output, "redaction_stage": "before_buffer"}


def _sampling_value(stream: str, salt: str, stable_key: str) -> tuple[int, str]:
    material = f"chill-sampling-v1\0{stream}\0{salt}\0{stable_key}".encode()
    digest = hashlib.sha256(material).digest()
    return int.from_bytes(digest[:8], "big"), digest.hex()[:16]


def _sampling_scenario(given: Mapping[str, object]) -> JsonObject:
    results: list[JsonObject] = []
    for case in given["cases"]:
        decisions: JsonObject = {}
        for stream, rate in given["streams"].items():
            value, prefix = _sampling_value(stream, given["salt"], case["stable_key"])
            numerator = int(rate["numerator"])
            denominator = int(rate["denominator"])
            if denominator <= 0 or not 0 <= numerator <= denominator:
                raise ConformanceSuiteError("sampling rate must be a bounded fraction")
            threshold = (1 << 64) * numerator // denominator
            decisions[stream] = {
                "kept": value < threshold,
                "digest_prefix": prefix,
            }
        results.append(
            {
                "case": case["case"],
                "stable_key": case["stable_key"],
                "trace_sampled": case["trace_sampled"],
                "decisions": decisions,
            }
        )
    return {"algorithm": "sha256-first-u64", "results": results}


_HANDLERS: dict[str, Callable[[Mapping[str, object]], JsonObject]] = {
    "annotations": _annotation_scenario,
    "navigation": _navigation_scenario,
    "actions": _actions_scenario,
    "trace": _trace_scenario,
    "offline": _offline_scenario,
    "replay": _replay_scenario,
    "schema": _schema_scenario,
    "redaction": _redaction_scenario,
    "sampling": _sampling_scenario,
}


def _compare_scenario(
    scenario: Mapping[str, object], actual: JsonObject, *, layer: str
) -> ScenarioResult:
    expected = scenario["expect"]
    reasons: list[str] = []
    if actual != expected:
        reasons.append("output_mismatch")
    serialized = _canonical_bytes(actual)
    for token in scenario.get("forbidden_tokens", []):
        if token.encode("utf-8") in serialized:
            reasons.append(f"forbidden_token:{token}")
    return ScenarioResult(
        scenario_id=scenario["id"],
        domain=scenario["domain"],
        layer=layer,
        passed=not reasons,
        expected_digest=_digest(expected),
        actual_digest=_digest(actual),
        reasons=tuple(reasons),
        actual=actual,
    )


def run_suite(manifest_path: Path) -> SuiteEvaluation:
    manifest, scenarios = load_suite(manifest_path)
    results = tuple(
        _compare_scenario(
            scenario,
            _HANDLERS[scenario["domain"]](scenario["given"]),
            layer="model",
        )
        for scenario in scenarios
    )
    return SuiteEvaluation(
        suite_version=_SUITE_VERSION,
        suite_digest=suite_digest(manifest, scenarios),
        passed=all(result.passed for result in results),
        results=results,
    )


def evaluate_implementation_report(
    manifest_path: Path, report: Mapping[str, object]
) -> ImplementationEvaluation:
    """Verify exact SDK outputs and reject skips, duplicates, and stale suites."""

    manifest, scenarios = load_suite(manifest_path)
    digest = suite_digest(manifest, scenarios)
    if (
        report.get("suite_version") != _SUITE_VERSION
        or report.get("suite_digest") != digest
    ):
        raise ConformanceReportError("report targets a different conformance suite")
    profile = report.get("profile")
    if not isinstance(profile, str):
        raise ConformanceReportError("report profile must be a string")
    try:
        required_ids = _resolve_profile_scenarios(manifest, profile)
    except ConformanceSuiteError as error:
        raise ConformanceReportError(str(error)) from error
    implementation = report.get("implementation")
    if not isinstance(implementation, dict) or not all(
        isinstance(implementation.get(key), str) and implementation[key]
        for key in ("platform", "adapter", "sdk_version", "layer")
    ):
        raise ConformanceReportError("implementation provenance is incomplete")
    layer = implementation["layer"]
    if layer not in _LAYERS:
        raise ConformanceReportError("implementation layer is invalid")
    platform = implementation["platform"]
    allowed_platforms = (
        {"apple", "android", "web"}
        if profile.startswith("client.")
        else {"server"}
    )
    if platform not in allowed_platforms:
        raise ConformanceReportError(
            f"platform {platform} cannot claim profile {profile}"
        )
    raw_results = report.get("results")
    if not isinstance(raw_results, list):
        raise ConformanceReportError("report results must be a list")
    by_id: dict[str, JsonObject] = {}
    for item in raw_results:
        if not isinstance(item, dict) or not isinstance(item.get("scenario_id"), str):
            raise ConformanceReportError("report result is malformed")
        scenario_id = item["scenario_id"]
        if scenario_id in by_id:
            raise ConformanceReportError(f"duplicate scenario result: {scenario_id}")
        if item.get("status") != "passed":
            raise ConformanceReportError(f"scenario may not be skipped: {scenario_id}")
        if not isinstance(item.get("output"), dict):
            raise ConformanceReportError(
                f"scenario output must be an object: {scenario_id}"
            )
        by_id[scenario_id] = item["output"]
    if set(by_id) != set(required_ids):
        missing = sorted(set(required_ids) - set(by_id))
        unknown = sorted(set(by_id) - set(required_ids))
        raise ConformanceReportError(
            f"report scenario set mismatch; missing={missing}, unknown={unknown}"
        )
    scenarios_by_id = {scenario["id"]: scenario for scenario in scenarios}
    unsupported = [
        scenario_id
        for scenario_id in required_ids
        if layer not in scenarios_by_id[scenario_id]["layers"]
    ]
    if unsupported:
        raise ConformanceReportError(
            f"report layer {layer} is unsupported by scenarios: {unsupported}"
        )
    results = tuple(
        _compare_scenario(
            scenarios_by_id[scenario_id], by_id[scenario_id], layer=layer
        )
        for scenario_id in required_ids
    )
    return ImplementationEvaluation(
        suite_version=_SUITE_VERSION,
        suite_digest=digest,
        profile=profile,
        implementation=dict(implementation),
        passed=all(result.passed for result in results),
        results=results,
    )
