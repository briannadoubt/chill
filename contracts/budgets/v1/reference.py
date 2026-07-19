"""Strict evaluator for release-blocking SDK budget reports."""

from __future__ import annotations

from dataclasses import asdict, dataclass
import json
import math
from pathlib import Path
from typing import Any, Mapping


JsonObject = dict[str, Any]
_COMPARISONS = {"lte", "gte", "eq"}
_CORE_CATEGORIES = {
    "package",
    "startup",
    "hot_path",
    "memory",
    "cpu",
    "network",
    "disk",
    "reliability",
    "privacy",
}


class BudgetCatalogError(ValueError):
    """The checked-in budget catalog is internally inconsistent."""


class BudgetReportError(ValueError):
    """A benchmark report is malformed or has unverifiable provenance."""


@dataclass(frozen=True)
class ResolvedBudget:
    id: str
    category: str
    unit: str
    statistic: str
    comparison: str
    threshold: float
    minimum_samples: int
    method: str
    physical_device_required: bool
    release_blocking: bool


@dataclass(frozen=True)
class BudgetResult:
    id: str
    passed: bool
    actual: float | None
    threshold: float
    comparison: str
    release_blocking: bool
    reason: str


@dataclass(frozen=True)
class BudgetEvaluation:
    catalog_version: str
    profile: str
    candidate: str
    baseline: str
    passed: bool
    results: tuple[BudgetResult, ...]

    def as_dict(self) -> JsonObject:
        return asdict(self)


def load_catalog(path: str | Path) -> JsonObject:
    with Path(path).open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise BudgetCatalogError("budget catalog must be an object")
    validate_catalog(value)
    return value


def _mapping(
    value: object, label: str, error_type: type[ValueError]
) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise error_type(f"{label} must be an object")
    return value


def _profile_lineage(
    profiles: Mapping[str, Any], profile_id: str
) -> tuple[str, ...]:
    lineage: list[str] = []
    current: str | None = profile_id
    while current is not None:
        if current in lineage:
            raise BudgetCatalogError(f"profile inheritance cycle at {current}")
        profile = _mapping(
            profiles.get(current), f"profile {current}", BudgetCatalogError
        )
        lineage.append(current)
        parent = profile.get("extends")
        if parent is not None and not isinstance(parent, str):
            raise BudgetCatalogError(f"profile {current} extends must be a string")
        current = parent
    return tuple(lineage)


def validate_catalog(catalog: Mapping[str, Any]) -> None:
    version = catalog.get("schema_version")
    if not isinstance(version, str) or not version:
        raise BudgetCatalogError("catalog requires schema_version")
    profiles = _mapping(catalog.get("profiles"), "profiles", BudgetCatalogError)
    methods = _mapping(catalog.get("methods"), "methods", BudgetCatalogError)
    budgets = catalog.get("budgets")
    if not isinstance(budgets, list) or not budgets:
        raise BudgetCatalogError("catalog requires a non-empty budgets list")

    for profile_id, raw_profile in profiles.items():
        if not isinstance(profile_id, str) or not profile_id:
            raise BudgetCatalogError("profile IDs must be non-empty strings")
        profile = _mapping(
            raw_profile, f"profile {profile_id}", BudgetCatalogError
        )
        if profile.get("platform") not in {"apple", "android", "web", "server"}:
            raise BudgetCatalogError(f"profile {profile_id} has invalid platform")
        if profile.get("feature_set") not in {"core", "replay"}:
            raise BudgetCatalogError(f"profile {profile_id} has invalid feature_set")
        lineage = _profile_lineage(profiles, profile_id)
        for ancestor_id in lineage[1:]:
            ancestor = _mapping(
                profiles[ancestor_id], f"profile {ancestor_id}", BudgetCatalogError
            )
            if ancestor.get("platform") != profile.get("platform"):
                raise BudgetCatalogError("profiles cannot extend another platform")

    for method_id, raw_method in methods.items():
        method = _mapping(raw_method, f"method {method_id}", BudgetCatalogError)
        if method.get("platform") not in {
            "apple",
            "android",
            "web",
            "server",
        }:
            raise BudgetCatalogError(f"method {method_id} has invalid platform")
        if not isinstance(method.get("physical_device"), bool):
            raise BudgetCatalogError(
                f"method {method_id} requires physical_device boolean"
            )
        if not isinstance(method.get("description"), str):
            raise BudgetCatalogError(f"method {method_id} requires description")

    seen: set[str] = set()
    for raw_budget in budgets:
        budget = _mapping(raw_budget, "budget", BudgetCatalogError)
        budget_id = budget.get("id")
        if not isinstance(budget_id, str) or not budget_id:
            raise BudgetCatalogError("budget requires an ID")
        if budget_id in seen:
            raise BudgetCatalogError(f"duplicate budget ID: {budget_id}")
        seen.add(budget_id)
        if budget.get("comparison") not in _COMPARISONS:
            raise BudgetCatalogError(f"budget {budget_id} has invalid comparison")
        if not isinstance(budget.get("category"), str):
            raise BudgetCatalogError(f"budget {budget_id} requires category")
        if not isinstance(budget.get("unit"), str) or not isinstance(
            budget.get("statistic"), str
        ):
            raise BudgetCatalogError(
                f"budget {budget_id} requires unit and statistic"
            )
        minimum_samples = budget.get("minimum_samples")
        if (
            not isinstance(minimum_samples, int)
            or isinstance(minimum_samples, bool)
            or minimum_samples < 1
        ):
            raise BudgetCatalogError(
                f"budget {budget_id} requires positive minimum_samples"
            )
        if budget.get("release_blocking") is not True:
            raise BudgetCatalogError(
                f"budget {budget_id} must be explicitly release blocking"
            )
        thresholds = _mapping(
            budget.get("thresholds"),
            f"budget {budget_id} thresholds",
            BudgetCatalogError,
        )
        method_by_platform = _mapping(
            budget.get("method_by_platform"),
            f"budget {budget_id} method_by_platform",
            BudgetCatalogError,
        )
        if not thresholds:
            raise BudgetCatalogError(f"budget {budget_id} has no thresholds")
        for profile_id, threshold in thresholds.items():
            if profile_id not in profiles:
                raise BudgetCatalogError(
                    f"budget {budget_id} names unknown profile {profile_id}"
                )
            if (
                not isinstance(threshold, (int, float))
                or isinstance(threshold, bool)
                or not math.isfinite(float(threshold))
            ):
                raise BudgetCatalogError(
                    f"budget {budget_id} threshold must be finite"
                )
            profile = _mapping(
                profiles[profile_id], f"profile {profile_id}", BudgetCatalogError
            )
            platform = str(profile["platform"])
            method_id = method_by_platform.get(platform)
            if method_id not in methods:
                raise BudgetCatalogError(
                    f"budget {budget_id} has no valid {platform} method"
                )
            method = _mapping(
                methods[method_id], f"method {method_id}", BudgetCatalogError
            )
            if method.get("platform") != platform:
                raise BudgetCatalogError(
                    f"budget {budget_id} method platform does not match profile"
                )

    for profile_id, raw_profile in profiles.items():
        profile = _mapping(
            raw_profile, f"profile {profile_id}", BudgetCatalogError
        )
        categories = {
            budget.category for budget in resolve_budgets(catalog, profile_id)
        }
        missing = _CORE_CATEGORIES - categories
        if missing:
            raise BudgetCatalogError(
                f"profile {profile_id} lacks categories: {sorted(missing)}"
            )
        if profile.get("feature_set") == "replay" and "replay" not in categories:
            raise BudgetCatalogError(f"profile {profile_id} lacks replay budgets")


def resolve_budgets(
    catalog: Mapping[str, Any], profile_id: str
) -> tuple[ResolvedBudget, ...]:
    profiles = _mapping(catalog.get("profiles"), "profiles", BudgetCatalogError)
    if profile_id not in profiles:
        raise BudgetCatalogError(f"unknown profile: {profile_id}")
    profile = _mapping(
        profiles[profile_id], f"profile {profile_id}", BudgetCatalogError
    )
    platform = str(profile["platform"])
    lineage = _profile_lineage(profiles, profile_id)
    methods = _mapping(catalog.get("methods"), "methods", BudgetCatalogError)
    raw_budgets = catalog.get("budgets")
    if not isinstance(raw_budgets, list):
        raise BudgetCatalogError("budgets must be a list")

    resolved: list[ResolvedBudget] = []
    for raw_budget in raw_budgets:
        budget = _mapping(raw_budget, "budget", BudgetCatalogError)
        thresholds = _mapping(
            budget.get("thresholds"), "budget thresholds", BudgetCatalogError
        )
        threshold: object | None = None
        for candidate in lineage:
            if candidate in thresholds:
                threshold = thresholds[candidate]
                break
        if threshold is None:
            continue
        method_by_platform = _mapping(
            budget.get("method_by_platform"),
            "budget method_by_platform",
            BudgetCatalogError,
        )
        method_id = method_by_platform.get(platform)
        if not isinstance(method_id, str):
            raise BudgetCatalogError(
                f"budget {budget.get('id')} has no method for {platform}"
            )
        method = _mapping(
            methods.get(method_id), f"method {method_id}", BudgetCatalogError
        )
        resolved.append(
            ResolvedBudget(
                id=str(budget["id"]),
                category=str(budget["category"]),
                unit=str(budget["unit"]),
                statistic=str(budget["statistic"]),
                comparison=str(budget["comparison"]),
                threshold=float(threshold),
                minimum_samples=int(budget["minimum_samples"]),
                method=method_id,
                physical_device_required=bool(method["physical_device"]),
                release_blocking=bool(budget["release_blocking"]),
            )
        )
    return tuple(sorted(resolved, key=lambda item: item.id))


def _passes(comparison: str, actual: float, threshold: float) -> bool:
    if comparison == "lte":
        return actual <= threshold
    if comparison == "gte":
        return actual >= threshold
    if comparison == "eq":
        return actual == threshold
    raise AssertionError(f"unsupported comparison: {comparison}")


def evaluate_report(
    catalog: Mapping[str, Any], report: Mapping[str, Any]
) -> BudgetEvaluation:
    validate_catalog(catalog)
    catalog_version = catalog["schema_version"]
    if report.get("catalog_version") != catalog_version:
        raise BudgetReportError("report catalog_version does not match")
    profile_id = report.get("profile")
    if not isinstance(profile_id, str):
        raise BudgetReportError("report requires a profile")
    profiles = _mapping(catalog.get("profiles"), "profiles", BudgetCatalogError)
    if profile_id not in profiles:
        raise BudgetReportError(f"report names unknown profile: {profile_id}")
    candidate = report.get("candidate")
    baseline = report.get("baseline")
    if not isinstance(candidate, str) or not candidate:
        raise BudgetReportError("report requires a candidate identity")
    if not isinstance(baseline, str) or not baseline:
        raise BudgetReportError("report requires a baseline identity")
    if candidate == baseline:
        raise BudgetReportError("candidate and baseline identities must differ")
    measurements = report.get("measurements")
    if not isinstance(measurements, list):
        raise BudgetReportError("report measurements must be a list")

    resolved = {budget.id: budget for budget in resolve_budgets(catalog, profile_id)}
    supplied: dict[str, Mapping[str, Any]] = {}
    for raw_measurement in measurements:
        measurement = _mapping(
            raw_measurement, "measurement", BudgetReportError
        )
        measurement_id = measurement.get("id")
        if not isinstance(measurement_id, str) or measurement_id not in resolved:
            raise BudgetReportError(f"unknown measurement: {measurement_id}")
        if measurement_id in supplied:
            raise BudgetReportError(f"duplicate measurement: {measurement_id}")
        supplied[measurement_id] = measurement

    results: list[BudgetResult] = []
    for budget_id, budget in sorted(resolved.items()):
        measurement = supplied.get(budget_id)
        if measurement is None:
            results.append(
                BudgetResult(
                    id=budget_id,
                    passed=False,
                    actual=None,
                    threshold=budget.threshold,
                    comparison=budget.comparison,
                    release_blocking=budget.release_blocking,
                    reason="required measurement is missing",
                )
            )
            continue
        if measurement.get("unit") != budget.unit:
            raise BudgetReportError(f"{budget_id} unit does not match catalog")
        if measurement.get("statistic") != budget.statistic:
            raise BudgetReportError(
                f"{budget_id} statistic does not match catalog"
            )
        if measurement.get("method") != budget.method:
            raise BudgetReportError(f"{budget_id} method does not match catalog")
        if measurement.get("build_mode") != "release":
            raise BudgetReportError(f"{budget_id} was not measured in release mode")
        if not isinstance(measurement.get("physical_device"), bool):
            raise BudgetReportError(
                f"{budget_id} physical_device provenance must be boolean"
            )
        if (
            budget.physical_device_required
            and measurement.get("physical_device") is not True
        ):
            raise BudgetReportError(
                f"{budget_id} requires a physical-device measurement"
            )
        samples = measurement.get("samples")
        if (
            not isinstance(samples, int)
            or isinstance(samples, bool)
            or samples < budget.minimum_samples
        ):
            raise BudgetReportError(
                f"{budget_id} has fewer than {budget.minimum_samples} samples"
            )
        raw_value = measurement.get("value")
        if (
            not isinstance(raw_value, (int, float))
            or isinstance(raw_value, bool)
            or not math.isfinite(float(raw_value))
        ):
            raise BudgetReportError(f"{budget_id} value must be finite")
        actual = float(raw_value)
        passed = _passes(budget.comparison, actual, budget.threshold)
        results.append(
            BudgetResult(
                id=budget_id,
                passed=passed,
                actual=actual,
                threshold=budget.threshold,
                comparison=budget.comparison,
                release_blocking=budget.release_blocking,
                reason="within budget" if passed else "threshold exceeded",
            )
        )

    passed = all(result.passed or not result.release_blocking for result in results)
    return BudgetEvaluation(
        catalog_version=str(catalog_version),
        profile=profile_id,
        candidate=candidate,
        baseline=baseline,
        passed=passed,
        results=tuple(results),
    )
