"""Strict, portable V2 release-budget contract for SDK artifacts."""

from __future__ import annotations

from dataclasses import asdict, dataclass
from datetime import datetime, timedelta, timezone
import json
import math
from pathlib import Path
from typing import Any, Mapping

JsonObject = dict[str, Any]
_COMPARISONS = {"lte", "gte", "eq"}
_REQUIRED_DIMENSIONS = {
    "package", "overhead", "memory", "cpu", "queue", "network",
    "reliability", "privacy", "ipc",
}
_V1_BINDING = {"catalog": "budgets/sdk/v1/budgets.json", "schema_version": "1.0.0"}


class BudgetCatalogError(ValueError):
    """The checked-in V2 catalog is invalid."""


class BudgetReportError(ValueError):
    """Release evidence cannot prove that it conforms to V2."""


@dataclass(frozen=True)
class ResolvedBudget:
    id: str; category: str; unit: str; statistic: str; comparison: str
    threshold: float; minimum_samples: int; method: str


@dataclass(frozen=True)
class BudgetResult:
    id: str; passed: bool; actual: float | None; threshold: float
    comparison: str; release_blocking: bool; reason: str


@dataclass(frozen=True)
class BudgetEvaluation:
    catalog_version: str; profile: str; candidate: str; baseline: str
    passed: bool; results: tuple[BudgetResult, ...]
    def as_dict(self) -> JsonObject: return asdict(self)


def _map(value: object, label: str, error: type[ValueError]) -> Mapping[str, Any]:
    if not isinstance(value, Mapping): raise error(f"{label} must be an object")
    return value


def _finite(value: object) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(float(value))


def load_catalog(path: str | Path) -> JsonObject:
    with Path(path).open(encoding="utf-8") as handle: catalog = json.load(handle)
    if not isinstance(catalog, dict): raise BudgetCatalogError("budget catalog must be an object")
    validate_catalog(catalog)
    return catalog


def validate_catalog(catalog: Mapping[str, Any]) -> None:
    if catalog.get("schema_version") != "2.0.0": raise BudgetCatalogError("V2 catalog requires schema_version 2.0.0")
    if catalog.get("binds_v1") != _V1_BINDING: raise BudgetCatalogError("V2 catalog must explicitly bind the V1 catalog")
    ttl = catalog.get("max_evidence_age_hours")
    if not isinstance(ttl, int) or isinstance(ttl, bool) or ttl < 1: raise BudgetCatalogError("max_evidence_age_hours must be positive")
    profiles = _map(catalog.get("profiles"), "profiles", BudgetCatalogError)
    methods = _map(catalog.get("methods"), "methods", BudgetCatalogError)
    budgets = catalog.get("budgets")
    if not isinstance(budgets, list) or not budgets: raise BudgetCatalogError("catalog requires budgets")
    for name, profile in profiles.items():
        data = _map(profile, f"profile {name}", BudgetCatalogError)
        if not isinstance(name, str) or not name or data.get("release_blocking") is not True: raise BudgetCatalogError(f"profile {name} must be release blocking")
    seen: set[str] = set(); coverage = {name: set() for name in profiles}
    for raw in budgets:
        budget = _map(raw, "budget", BudgetCatalogError); ident = budget.get("id")
        if not isinstance(ident, str) or not ident or ident in seen: raise BudgetCatalogError(f"duplicate or invalid budget ID: {ident}")
        seen.add(ident)
        if budget.get("release_blocking") is not True: raise BudgetCatalogError(f"budget {ident} must be release blocking")
        if budget.get("comparison") not in _COMPARISONS: raise BudgetCatalogError(f"budget {ident} has invalid comparison")
        if not isinstance(budget.get("category"), str) or not isinstance(budget.get("unit"), str) or not isinstance(budget.get("statistic"), str): raise BudgetCatalogError(f"budget {ident} needs category, unit, statistic")
        samples = budget.get("minimum_samples")
        if not isinstance(samples, int) or isinstance(samples, bool) or samples < 1: raise BudgetCatalogError(f"budget {ident} has invalid minimum_samples")
        thresholds = _map(budget.get("thresholds"), f"budget {ident} thresholds", BudgetCatalogError)
        by_profile = _map(budget.get("method_by_profile"), f"budget {ident} methods", BudgetCatalogError)
        if set(thresholds) != set(profiles) or set(by_profile) != set(profiles): raise BudgetCatalogError(f"budget {ident} must cover every V2 profile")
        for profile in profiles:
            if not _finite(thresholds[profile]): raise BudgetCatalogError(f"budget {ident} threshold must be finite")
            method = by_profile[profile]
            if method not in methods: raise BudgetCatalogError(f"budget {ident} has unknown method for {profile}")
            coverage[profile].add(str(budget["category"]))
    for profile, dimensions in coverage.items():
        missing = _REQUIRED_DIMENSIONS - dimensions
        if missing: raise BudgetCatalogError(f"profile {profile} lacks dimensions: {sorted(missing)}")


def resolve_budgets(catalog: Mapping[str, Any], profile: str) -> tuple[ResolvedBudget, ...]:
    validate_catalog(catalog); profiles = _map(catalog["profiles"], "profiles", BudgetCatalogError)
    if profile not in profiles: raise BudgetCatalogError(f"unknown profile: {profile}")
    return tuple(sorted((ResolvedBudget(str(b["id"]), str(b["category"]), str(b["unit"]), str(b["statistic"]), str(b["comparison"]), float(b["thresholds"][profile]), int(b["minimum_samples"]), str(b["method_by_profile"][profile])) for b in catalog["budgets"]), key=lambda b: b.id))


def _timestamp(value: object) -> datetime:
    if not isinstance(value, str): raise BudgetReportError("report requires generated_at")
    try: parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error: raise BudgetReportError("generated_at must be ISO-8601") from error
    if parsed.tzinfo is None: raise BudgetReportError("generated_at must include timezone")
    return parsed.astimezone(timezone.utc)


def evaluate_report(catalog: Mapping[str, Any], report: Mapping[str, Any], *, now: datetime | None = None) -> BudgetEvaluation:
    validate_catalog(catalog)
    if report.get("catalog_version") != catalog["schema_version"]: raise BudgetReportError("report catalog_version does not match")
    profile = report.get("profile"); profiles = _map(catalog["profiles"], "profiles", BudgetCatalogError)
    if not isinstance(profile, str) or profile not in profiles: raise BudgetReportError("report names unknown profile")
    candidate, baseline = report.get("candidate"), report.get("baseline")
    if not isinstance(candidate, str) or not candidate or not isinstance(baseline, str) or not baseline or candidate == baseline: raise BudgetReportError("report requires distinct candidate and baseline")
    created = _timestamp(report.get("generated_at")); clock = (now or datetime.now(timezone.utc)).astimezone(timezone.utc)
    if created > clock + timedelta(minutes=5) or clock - created > timedelta(hours=int(catalog["max_evidence_age_hours"])): raise BudgetReportError("report evidence is stale")
    measurements = report.get("measurements")
    if not isinstance(measurements, list): raise BudgetReportError("report measurements must be a list")
    resolved = {b.id: b for b in resolve_budgets(catalog, profile)}; supplied: dict[str, Mapping[str, Any]] = {}
    for raw in measurements:
        item = _map(raw, "measurement", BudgetReportError); ident = item.get("id")
        if not isinstance(ident, str) or ident not in resolved: raise BudgetReportError(f"unknown measurement: {ident}")
        if ident in supplied: raise BudgetReportError(f"duplicate measurement: {ident}")
        if item.get("profile") != profile: raise BudgetReportError(f"{ident} profile does not match report profile")
        supplied[ident] = item
    results: list[BudgetResult] = []
    for ident, budget in sorted(resolved.items()):
        item = supplied.get(ident)
        if item is None: results.append(BudgetResult(ident, False, None, budget.threshold, budget.comparison, True, "required measurement is missing")); continue
        if item.get("unit") != budget.unit or item.get("statistic") != budget.statistic or item.get("method") != budget.method or item.get("build_mode") != "release": raise BudgetReportError(f"{ident} provenance does not match catalog")
        samples = item.get("samples")
        if not isinstance(samples, int) or isinstance(samples, bool) or samples < budget.minimum_samples: raise BudgetReportError(f"{ident} has fewer than {budget.minimum_samples} samples")
        value = item.get("value")
        if not _finite(value): raise BudgetReportError(f"{ident} value must be finite")
        actual = float(value); passed = {"lte": actual <= budget.threshold, "gte": actual >= budget.threshold, "eq": actual == budget.threshold}[budget.comparison]
        results.append(BudgetResult(ident, passed, actual, budget.threshold, budget.comparison, True, "within budget" if passed else "threshold exceeded"))
    return BudgetEvaluation(str(catalog["schema_version"]), profile, candidate, baseline, all(r.passed for r in results), tuple(results))
