"""Executable V2 portable conformance protocol, layered over the immutable V1 suite."""
from __future__ import annotations

from dataclasses import asdict, dataclass
import hashlib
import json
from pathlib import Path
from typing import Any, Mapping, Sequence

from contracts.conformance.v1 import evaluate_implementation_report as evaluate_v1
from contracts.conformance.v1 import load_suite as load_v1
from contracts.conformance.v1 import suite_digest as v1_suite_digest

JsonObject = dict[str, Any]
_VERSION = "2.0.0"
_LAYERS = {"model", "adapter", "wire"}
_PROFILES = {
    "portable.rust": ("server.core", "server", "rust"),
    "server.javascript": ("server.core", "server", "javascript"),
    "desktop.electron.host": ("server.core", "server", "electron-host"),
    "desktop.electron.renderer": ("client.core", "web", "electron-renderer"),
    "desktop.tauri.host": ("server.core", "server", "tauri-host"),
    "desktop.tauri.webview": ("client.core", "web", "tauri-webview"),
}

class ConformanceSuiteError(ValueError): pass
class ConformanceReportError(ValueError): pass

@dataclass(frozen=True)
class ScenarioResult:
    scenario_id: str; domain: str; layer: str; passed: bool; expected_digest: str; actual_digest: str; reasons: tuple[str, ...]; actual: JsonObject
    def as_dict(self) -> JsonObject: return asdict(self)
@dataclass(frozen=True)
class SuiteEvaluation:
    suite_version: str; suite_digest: str; passed: bool; results: tuple[ScenarioResult, ...]
    def as_dict(self) -> JsonObject: return asdict(self)
@dataclass(frozen=True)
class ImplementationEvaluation:
    suite_version: str; suite_digest: str; profile: str; implementation: JsonObject; passed: bool; results: tuple[ScenarioResult, ...]
    def as_dict(self) -> JsonObject: return asdict(self)

def _bytes(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()
def _digest(value: object) -> str: return hashlib.sha256(_bytes(value)).hexdigest()
def _object(value: object, label: str, error: type[ValueError] = ConformanceSuiteError) -> JsonObject:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value): raise error(f"{label} must be a JSON object")
    return value
def _load(path: Path) -> JsonObject:
    try:
        with path.open(encoding="utf-8") as file: return _object(json.load(file), str(path))
    except (OSError, json.JSONDecodeError) as error: raise ConformanceSuiteError(f"cannot load {path}: {error}") from error

def load_suite(manifest_path: Path) -> tuple[JsonObject, tuple[JsonObject, ...]]:
    path = manifest_path.resolve(); manifest = _load(path)
    if manifest.get("suite_version") != _VERSION: raise ConformanceSuiteError(f"suite version must be {_VERSION}")
    binding = _object(manifest.get("v1_binding"), "v1_binding")
    v1_path = path.parents[1] / "v1" / "manifest.json"
    v1_manifest, v1_scenarios = load_v1(v1_path)
    if binding != {"suite_version": v1_manifest["suite_version"], "suite_digest": v1_suite_digest(v1_manifest, v1_scenarios)}:
        raise ConformanceSuiteError("V2 must bind the exact checked V1 suite version and digest")
    profiles = _object(manifest.get("profiles"), "profiles")
    if set(profiles) != set(_PROFILES): raise ConformanceSuiteError("manifest must define every portable V2 profile")
    descriptors = manifest.get("scenarios")
    if not isinstance(descriptors, list) or not descriptors: raise ConformanceSuiteError("manifest scenarios must be nonempty")
    scenarios=[]; ids=set(); root=path.parent
    for raw in descriptors:
        d=_object(raw, "scenario descriptor"); sid=d.get("id"); domain=d.get("domain"); layers=d.get("layers"); relative=d.get("path")
        if not isinstance(sid,str) or not isinstance(domain,str) or not isinstance(relative,str) or not sid: raise ConformanceSuiteError("scenario descriptor fields must be strings")
        if sid in ids: raise ConformanceSuiteError(f"duplicate scenario descriptor: {sid}")
        if not isinstance(layers,list) or set(layers)!=_LAYERS or len(layers)!=3: raise ConformanceSuiteError(f"invalid layers for {sid}")
        scenario_path=(root/relative).resolve()
        if not scenario_path.is_relative_to(root): raise ConformanceSuiteError("scenario path escapes the suite directory")
        scenario=_load(scenario_path)
        if any(scenario.get(k)!=d[k] for k in ("id","domain","layers")) or scenario.get("suite_version")!=_VERSION: raise ConformanceSuiteError(f"scenario {sid} does not match descriptor")
        _object(scenario.get("given"), f"{sid} given"); _object(scenario.get("expect"), f"{sid} expect")
        tokens=scenario.get("forbidden_tokens", [])
        if not isinstance(tokens,list) or not all(isinstance(x,str) and x for x in tokens): raise ConformanceSuiteError(f"{sid} forbidden_tokens must be strings")
        ids.add(sid); scenarios.append(scenario)
    for profile, config in profiles.items():
        expected_v1, platform, adapter = _PROFILES[profile]
        if _object(config, f"profile {profile}") != {"v1_profile": expected_v1, "platform": platform, "adapter": adapter, "scenarios": config.get("scenarios")}:
            raise ConformanceSuiteError(f"profile {profile} is stale or malformed")
        listed=config["scenarios"]
        if not isinstance(listed,list) or len(listed)!=len(set(listed)) or set(listed)!=ids: raise ConformanceSuiteError(f"profile {profile} must require every portable scenario exactly once")
    return manifest, tuple(scenarios)

def suite_digest(manifest: Mapping[str, object], scenarios: Sequence[Mapping[str, object]]) -> str: return _digest({"manifest":manifest,"scenarios":list(scenarios)})

def _actual(scenario: Mapping[str, object]) -> JsonObject:
    g=_object(scenario["given"], "given"); sid=scenario["id"]
    if sid == "ipc.metadata_correlation":
        trusted=g["trusted_message"]; untrusted=g["untrusted_message"]; malformed=g["malformed_message"]
        return {"trusted":{"traceparent":trusted["traceparent"],"correlation_id":trusted["correlation_id"],"accepted":True},"untrusted":{"traceparent":None,"correlation_id":None,"accepted":False},"malformed":{"traceparent":None,"correlation_id":None,"accepted":False,"reason":"malformed_correlation"}}
    if sid == "async.producer_consumer_links":
        return {"consumer_parent":g["consumer_ambient"],"producer_links":[g["producer_context"]],"link_count":1}
    if sid == "attributes.bounded_default_deny":
        allowed=g["attributes"]["public.label"][:g["max_value_chars"]]
        return {"attributes":{"public.label":allowed},"dropped_keys":["private.token","unknown.debug"],"policy":"default_deny","max_value_chars":g["max_value_chars"]}
    if sid == "offline.retry_identity":
        ids=[item["record_id"] for item in g["records"]]
        return {"attempts":[ids,ids],"retry_identity":[item["envelope_id"] for item in g["records"]],"acknowledged":ids,"at_least_once":True}
    if sid == "privacy.ipc_canary_exclusion":
        return {"forwarded":{"event":"renderer.ready","metadata":{"surface_id":"settings"}},"excluded_fields":["authorization","user_input"]}
    raise ConformanceSuiteError(f"unknown V2 scenario: {sid}")

def _result(s: Mapping[str, object], actual: JsonObject, layer: str) -> ScenarioResult:
    expected=_object(s["expect"], "expect"); reasons=[]
    if actual != expected: reasons.append("output_mismatch")
    text=json.dumps(actual, sort_keys=True)
    reasons.extend(f"forbidden_token:{token}" for token in s.get("forbidden_tokens",[]) if token in text)
    return ScenarioResult(s["id"],s["domain"],layer,not reasons,_digest(expected),_digest(actual),tuple(reasons),actual)

def run_suite(manifest_path: Path) -> SuiteEvaluation:
    manifest, scenarios=load_suite(manifest_path); digest=suite_digest(manifest,scenarios)
    results=tuple(_result(s,_actual(s),"model") for s in scenarios)
    return SuiteEvaluation(_VERSION,digest,all(r.passed for r in results),results)

def evaluate_implementation_report(manifest_path: Path, report: Mapping[str, object]) -> ImplementationEvaluation:
    manifest, scenarios=load_suite(manifest_path); digest=suite_digest(manifest,scenarios); report=_object(report,"report",ConformanceReportError)
    if report.get("suite_version") != _VERSION or report.get("suite_digest") != digest: raise ConformanceReportError("report binds a different conformance suite")
    profile=report.get("profile")
    if profile not in _PROFILES: raise ConformanceReportError("report profile is invalid")
    implementation=_object(report.get("implementation"),"implementation",ConformanceReportError); expected_v1, platform, adapter=_PROFILES[profile]
    for key in ("platform","adapter","sdk_version","layer"):
        if not isinstance(implementation.get(key),str) or not implementation[key]: raise ConformanceReportError("implementation provenance is incomplete")
    if implementation["platform"] != platform or implementation["adapter"] != adapter: raise ConformanceReportError("implementation cannot claim this profile provenance")
    if implementation["layer"] not in _LAYERS: raise ConformanceReportError("implementation layer is invalid")
    v1_report=_object(report.get("v1_report"),"v1_report",ConformanceReportError)
    if v1_report.get("profile") != expected_v1: raise ConformanceReportError("V1 report profile does not match portable profile")
    try: v1_evaluation=evaluate_v1(manifest_path.resolve().parents[1]/"v1"/"manifest.json", v1_report)
    except ValueError as error: raise ConformanceReportError(f"V1 report is invalid: {error}") from error
    if not v1_evaluation.passed: raise ConformanceReportError("applicable V1 profile did not pass")
    raw=report.get("results")
    if not isinstance(raw,list): raise ConformanceReportError("results must be a list")
    by_id={s["id"]:s for s in scenarios}; seen=set(); results=[]
    for item in raw:
        item=_object(item,"result",ConformanceReportError); sid=item.get("scenario_id")
        if not isinstance(sid,str) or sid not in by_id or sid in seen: raise ConformanceReportError("duplicate scenario or scenario set mismatch")
        seen.add(sid)
        if item.get("status") != "passed": raise ConformanceReportError("scenario may not be skipped")
        output=_object(item.get("output"),f"output {sid}",ConformanceReportError)
        results.append(_result(by_id[sid],output,implementation["layer"]))
    if seen != set(by_id): raise ConformanceReportError("scenario set mismatch")
    return ImplementationEvaluation(_VERSION,digest,profile,implementation,all(x.passed for x in results),tuple(results))
