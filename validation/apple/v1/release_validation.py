"""Deterministic planning and fail-closed Apple release evidence aggregation.

The collector-facing input is deliberately small: every JSON fragment binds
itself to the checked manifest and budget catalog, supplies immutable build and
device provenance, names one or more SHA-256 verified artifacts, and contains
scalar or paired observations.  This module never estimates missing evidence.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import random
import statistics
import subprocess
import tempfile
from typing import Any, Iterable, Mapping, Sequence

from contracts.budgets.v1.reference import load_catalog, resolve_budgets


JsonObject = dict[str, Any]
EXPECTED_PROFILE = "apple.replay"
EXPECTED_MEASUREMENT_COUNT = 40
_HEX = frozenset("0123456789abcdef")
_BUILD_FIELDS = (
    "candidate",
    "baseline",
    "revision",
    "configuration",
    "xcode_version",
    "swift_version",
)
_DEVICE_FIELDS = (
    "class",
    "model",
    "os_version",
    "physical",
    "thermal_state",
    "power",
)
_SENSITIVE_KEYS = {
    "device_id",
    "udid",
    "identifier",
    "development_team",
    "team_id",
    "provisioning_profile",
    "serial_number",
}


class ReleaseValidationError(ValueError):
    """Release evidence or its validation manifest is invalid."""


@dataclass(frozen=True)
class MeasurementSpec:
    id: str
    unit: str
    statistic: str
    minimum_samples: int
    sample_shape: str
    method: str
    physical_device_required: bool
    scenario_id: str
    required_artifacts: tuple[str, ...]


def load_json_object(path: str | Path, label: str) -> JsonObject:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ReleaseValidationError(f"cannot read {label}: {error}") from error
    if not isinstance(value, dict):
        raise ReleaseValidationError(f"{label} must be a JSON object")
    return value


def sha256_file(path: str | Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def sha256_artifact(path: str | Path) -> tuple[str, int]:
    """Hash a file or directory without following symlinks.

    Xcode result bundles and Instruments traces are directories. Their digest
    binds sorted relative paths, file lengths, and contents, making the result
    independent of filesystem enumeration order and timestamps.
    """
    artifact = Path(path)
    if artifact.is_symlink():
        raise ReleaseValidationError(f"artifact may not be a symlink: {artifact}")
    if artifact.is_file():
        return sha256_file(artifact), artifact.stat().st_size
    if not artifact.is_dir():
        raise ReleaseValidationError(f"artifact does not exist: {artifact}")
    digest = hashlib.sha256()
    total = 0
    files = sorted(item for item in artifact.rglob("*") if item.is_file())
    for item in files:
        if item.is_symlink():
            raise ReleaseValidationError(f"artifact contains a symlink: {item}")
        relative = item.relative_to(artifact).as_posix().encode("utf-8")
        size = item.stat().st_size
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(size.to_bytes(8, "big"))
        with item.open("rb") as handle:
            for block in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(block)
        total += size
    return digest.hexdigest(), total


def _canonical_sha256(value: object) -> str:
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _require_string(value: object, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ReleaseValidationError(f"{label} must be a non-empty string")
    return value


def _require_positive_int(value: object, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise ReleaseValidationError(f"{label} must be a positive integer")
    return value


def _require_digest(value: object, label: str) -> str:
    digest = _require_string(value, label).lower()
    if len(digest) != 64 or any(character not in _HEX for character in digest):
        raise ReleaseValidationError(f"{label} must be a lowercase SHA-256")
    return digest


def _required_provenance_fields(
    manifest: Mapping[str, Any], kind: str, defaults: Sequence[str]
) -> tuple[str, ...]:
    requirements = manifest.get("provenance_requirements", {})
    # Schema v2 names the complete evidence contract as a flat list. Build
    # and device fields remain deliberately normalized by this module.
    if isinstance(requirements, list):
        if not all(isinstance(field, str) and field for field in requirements):
            raise ReleaseValidationError("provenance_requirements must list strings")
        return tuple(defaults)
    if not isinstance(requirements, Mapping):
        raise ReleaseValidationError("provenance_requirements must be an object or list")
    raw = requirements.get(kind)
    if raw is None:
        raw = requirements.get(f"{kind}_fields")
    if raw is None:
        return tuple(defaults)
    if isinstance(raw, Mapping):
        raw = raw.get("required", raw.get("fields"))
    if not isinstance(raw, list) or not all(
        isinstance(field, str) and field for field in raw
    ):
        raise ReleaseValidationError(
            f"provenance_requirements.{kind} must list required fields"
        )
    return tuple(raw)


def load_manifest(path: str | Path) -> JsonObject:
    manifest = load_json_object(path, "Apple release manifest")
    _require_string(manifest.get("schema_version"), "manifest schema_version")
    if manifest.get("profile") != EXPECTED_PROFILE:
        raise ReleaseValidationError("manifest profile must be apple.replay")
    _require_string(manifest.get("budget_catalog"), "manifest budget_catalog")
    interleaving = manifest.get("interleaving")
    if not isinstance(interleaving, Mapping):
        raise ReleaseValidationError("manifest interleaving must be an object")
    algorithm = interleaving.get("algorithm")
    if algorithm is not None and algorithm != "sha256-seeded-fisher-yates-v1":
        raise ReleaseValidationError("manifest names an unsupported interleaving algorithm")
    if interleaving.get("allow_sequential_variant_batches") is True:
        raise ReleaseValidationError("sequential variant batches are forbidden")
    if interleaving.get("randomize_each_pair") is False:
        raise ReleaseValidationError("each baseline/candidate pair must be randomized")
    scenarios = manifest.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        raise ReleaseValidationError("manifest scenarios must be a non-empty list")
    floor = manifest.get("floor_device")
    if not isinstance(floor, Mapping):
        raise ReleaseValidationError("manifest floor_device must be an object")
    for field in ("class", "minimum_os", "thermal_state", "power"):
        _require_string(floor.get(field), f"floor_device.{field}")
    allowed_models = floor.get("allowed_models")
    if not isinstance(allowed_models, list) or not allowed_models or not all(
        isinstance(model, str) and model for model in allowed_models
    ):
        raise ReleaseValidationError(
            "floor_device.allowed_models must be a non-empty string list"
        )
    return manifest


def measurement_specs(
    manifest: Mapping[str, Any], catalog: Mapping[str, Any]
) -> dict[str, MeasurementSpec]:
    resolved = {budget.id: budget for budget in resolve_budgets(catalog, EXPECTED_PROFILE)}
    if len(resolved) != EXPECTED_MEASUREMENT_COUNT:
        raise ReleaseValidationError(
            f"apple.replay must resolve to exactly {EXPECTED_MEASUREMENT_COUNT} measurements"
        )
    scenarios = manifest.get("scenarios")
    if not isinstance(scenarios, list):
        raise ReleaseValidationError("manifest scenarios must be a list")
    supplied: dict[str, MeasurementSpec] = {}
    scenario_ids: set[str] = set()
    for raw_scenario in scenarios:
        if not isinstance(raw_scenario, Mapping):
            raise ReleaseValidationError("each scenario must be an object")
        scenario_id = _require_string(raw_scenario.get("id"), "scenario id")
        if scenario_id in scenario_ids:
            raise ReleaseValidationError(f"duplicate scenario: {scenario_id}")
        scenario_ids.add(scenario_id)
        method = _require_string(raw_scenario.get("method"), f"{scenario_id} method")
        _require_string(raw_scenario.get("collector"), f"{scenario_id} collector")
        if not isinstance(raw_scenario.get("physical_device_required"), bool):
            raise ReleaseValidationError(
                f"{scenario_id} physical_device_required must be boolean"
            )
        if not isinstance(raw_scenario.get("paired"), bool):
            raise ReleaseValidationError(f"{scenario_id} paired must be boolean")
        _require_positive_int(raw_scenario.get("minimum_runs"), f"{scenario_id} minimum_runs")
        if not isinstance(raw_scenario.get("workload"), (str, Mapping)):
            raise ReleaseValidationError(f"{scenario_id} workload is required")
        measurements = raw_scenario.get("measurements")
        if not isinstance(measurements, list) or not measurements:
            raise ReleaseValidationError(f"{scenario_id} measurements must be non-empty")
        raw_required_artifacts = raw_scenario.get("required_artifacts", [])
        if not isinstance(raw_required_artifacts, list) or not all(
            isinstance(name, str) and name for name in raw_required_artifacts
        ):
            raise ReleaseValidationError(
                f"{scenario_id} required_artifacts must list strings"
            )
        required_artifacts = tuple(raw_required_artifacts)
        for raw_measurement in measurements:
            if not isinstance(raw_measurement, Mapping):
                raise ReleaseValidationError("measurement spec must be an object")
            measurement_id = _require_string(raw_measurement.get("id"), "measurement id")
            if measurement_id in supplied:
                raise ReleaseValidationError(f"duplicate measurement: {measurement_id}")
            budget = resolved.get(measurement_id)
            if budget is None:
                raise ReleaseValidationError(f"unknown measurement: {measurement_id}")
            shape = _require_string(
                raw_measurement.get("sample_shape"), f"{measurement_id} sample_shape"
            )
            if shape not in {"scalar", "pair"}:
                raise ReleaseValidationError(
                    f"{measurement_id} sample_shape must be scalar or pair"
                )
            values = {
                "unit": raw_measurement.get("unit"),
                "statistic": raw_measurement.get("statistic"),
                "minimum_samples": raw_measurement.get("minimum_samples"),
                "method": method,
                "physical": raw_scenario.get("physical_device_required"),
            }
            expected = {
                "unit": budget.unit,
                "statistic": budget.statistic,
                "minimum_samples": budget.minimum_samples,
                "method": budget.method,
                "physical": budget.physical_device_required,
            }
            if values != expected:
                raise ReleaseValidationError(
                    f"manifest metadata does not match catalog for {measurement_id}"
                )
            if budget.statistic.endswith("paired_delta") or budget.statistic == "max_paired_delta":
                if shape != "pair":
                    raise ReleaseValidationError(f"{measurement_id} requires pair samples")
            supplied[measurement_id] = MeasurementSpec(
                id=measurement_id,
                unit=budget.unit,
                statistic=budget.statistic,
                minimum_samples=budget.minimum_samples,
                sample_shape=shape,
                method=budget.method,
                physical_device_required=budget.physical_device_required,
                scenario_id=scenario_id,
                required_artifacts=required_artifacts,
            )
    missing = sorted(set(resolved) - set(supplied))
    extra = sorted(set(supplied) - set(resolved))
    if missing or extra or len(supplied) != EXPECTED_MEASUREMENT_COUNT:
        raise ReleaseValidationError(
            f"manifest must define exactly the 40 apple.replay measurements; missing={missing}, extra={extra}"
        )
    return supplied


def validate_inputs(
    manifest_path: str | Path, catalog_path: str | Path | None = None
) -> tuple[JsonObject, JsonObject, dict[str, MeasurementSpec], str, str]:
    manifest_path = Path(manifest_path).resolve()
    manifest = load_manifest(manifest_path)
    if catalog_path is None:
        declared = Path(str(manifest["budget_catalog"]))
        catalog_path = declared if declared.is_absolute() else manifest_path.parents[3] / declared
    catalog_path = Path(catalog_path).resolve()
    catalog = load_catalog(catalog_path)
    specs = measurement_specs(manifest, catalog)
    manifest_digest = sha256_file(manifest_path)
    catalog_digest = sha256_file(catalog_path)
    declared_digest = manifest.get("budget_catalog_sha256")
    if declared_digest is not None and _require_digest(
        declared_digest, "manifest budget_catalog_sha256"
    ) != catalog_digest:
        raise ReleaseValidationError("manifest budget catalog SHA-256 does not match")
    return manifest, catalog, specs, manifest_digest, catalog_digest


def generate_plan(
    manifest: Mapping[str, Any], manifest_digest: str, catalog_digest: str, seed: int
) -> JsonObject:
    if not isinstance(seed, int) or isinstance(seed, bool):
        raise ReleaseValidationError("seed must be an integer")
    rng = _Sha256Rng(seed)
    tasks: list[JsonObject] = []
    scenarios = list(manifest["scenarios"])
    supplemental = manifest.get("supplemental_scenarios", [])
    if not isinstance(supplemental, list):
        raise ReleaseValidationError("supplemental_scenarios must be a list")
    scenarios.extend(supplemental)
    for raw_scenario in scenarios:
        assert isinstance(raw_scenario, Mapping)
        scenario_id = str(raw_scenario["id"])
        run_count = int(raw_scenario["minimum_runs"])
        paired = bool(raw_scenario["paired"])
        blocks: list[list[JsonObject]] = []
        for run_index in range(1, run_count + 1):
            variants = ["baseline", "candidate"] if paired else ["candidate"]
            if paired and rng.randrange(2):
                variants.reverse()
            blocks.append(
                [
                    {
                        "scenario": scenario_id,
                        "run": run_index,
                        "variant": variant,
                        "pair": run_index if paired else None,
                    }
                    for variant in variants
                ]
            )
        rng.shuffle(blocks)
        for block in blocks:
            tasks.extend(block)
    for sequence, task in enumerate(tasks, 1):
        task["sequence"] = sequence
    plan: JsonObject = {
        "schema_version": "1.0.0",
        "profile": EXPECTED_PROFILE,
        "seed": seed,
        "manifest_sha256": manifest_digest,
        "profiler_manifest_sha256": manifest_digest,
        "catalog_sha256": catalog_digest,
        "budget_catalog_sha256": catalog_digest,
        "tasks": tasks,
    }
    plan["execution_plan_sha256"] = _canonical_sha256(plan)
    plan["plan_id"] = plan["execution_plan_sha256"]
    return plan


class _Sha256Rng:
    """Tiny stable RNG for the manifest's SHA-256 Fisher-Yates algorithm."""

    def __init__(self, seed: int) -> None:
        self._seed = str(seed).encode("ascii")
        self._counter = 0

    def randrange(self, upper: int) -> int:
        if upper < 1:
            raise ValueError("upper bound must be positive")
        material = self._seed + b":" + str(self._counter).encode("ascii")
        self._counter += 1
        return int.from_bytes(hashlib.sha256(material).digest(), "big") % upper

    def shuffle(self, values: list[Any]) -> None:
        for index in range(len(values) - 1, 0, -1):
            other = self.randrange(index + 1)
            values[index], values[other] = values[other], values[index]


def _finite_number(value: object, label: str) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise ReleaseValidationError(f"{label} must be numeric")
    number = float(value)
    if not math.isfinite(number):
        raise ReleaseValidationError(f"{label} must be finite")
    return number


def _percentile(values: Sequence[float], percentile: float) -> float:
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    position = (len(ordered) - 1) * percentile
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    weight = position - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


def _upper_ci_mean(values: Sequence[float]) -> float:
    mean = statistics.fmean(values)
    if len(values) == 1:
        return mean
    # A normal 95% upper confidence bound is deterministic and conservative
    # for the release suite's minimum sample sizes. Five-run scenarios use the
    # Student-t 95% one-sided critical value.
    one_sided_t95 = (
        6.314, 2.920, 2.353, 2.132, 2.015, 1.943, 1.895, 1.860,
        1.833, 1.812, 1.796, 1.782, 1.771, 1.761, 1.753, 1.746,
        1.740, 1.734, 1.729, 1.725, 1.721, 1.717, 1.714, 1.711,
        1.708, 1.706, 1.703, 1.701, 1.699,
    )
    critical = one_sided_t95[min(len(values) - 2, len(one_sided_t95) - 1)]
    return mean + critical * statistics.stdev(values) / math.sqrt(len(values))


def _bootstrap_upper_percentile(
    values: Sequence[float], percentile: float, identity: str
) -> float:
    if len(values) == 1:
        return values[0]
    seed = int.from_bytes(hashlib.sha256(identity.encode("utf-8")).digest()[:8], "big")
    rng = random.Random(seed)
    estimates = []
    for _ in range(4096):
        sample = [values[rng.randrange(len(values))] for _ in values]
        estimates.append(_percentile(sample, percentile))
    return _percentile(estimates, 0.95)


def aggregate_statistic(statistic: str, values: Sequence[float], identity: str) -> float:
    if not values:
        raise ReleaseValidationError(f"{identity} has no samples")
    if statistic in {"max", "configured_max", "configured_value", "max_paired_delta"}:
        return max(values)
    if statistic in {
        "total",
        "total_after_denial",
        "total_at_rated_load",
    }:
        return sum(values)
    if statistic == "p95":
        return _percentile(values, 0.95)
    if statistic == "p99":
        return _percentile(values, 0.99)
    if statistic == "upper_ci_mean":
        return _upper_ci_mean(values)
    if statistic == "upper_ci_mean_paired_delta":
        return _upper_ci_mean(values)
    if statistic == "upper_ci_p95_paired_delta":
        return _bootstrap_upper_percentile(values, 0.95, identity)
    if statistic == "upper_ci_p99_paired_delta":
        return _bootstrap_upper_percentile(values, 0.99, identity)
    raise ReleaseValidationError(f"unsupported statistic for {identity}: {statistic}")


def _sanitize_provenance(
    value: object, fields: Sequence[str], label: str
) -> JsonObject:
    if not isinstance(value, Mapping):
        raise ReleaseValidationError(f"{label} provenance must be an object")
    lowered = {str(key).lower() for key in value}
    forbidden = sorted(lowered & _SENSITIVE_KEYS)
    if forbidden:
        raise ReleaseValidationError(f"{label} provenance contains sensitive fields: {forbidden}")
    result: JsonObject = {}
    for field in fields:
        raw = value.get(field)
        if field == "physical":
            if not isinstance(raw, bool):
                raise ReleaseValidationError(f"{label}.{field} must be boolean")
            result[field] = raw
        else:
            result[field] = _require_string(raw, f"{label}.{field}")
    return result


def _validate_floor_device(
    manifest: Mapping[str, Any], device: Mapping[str, Any], fragment_path: Path
) -> None:
    if device.get("physical") is not True:
        return
    floor = manifest.get("floor_device")
    if not isinstance(floor, Mapping):
        raise ReleaseValidationError("manifest floor_device must be an object")
    expected = {
        "class": floor.get("class"),
        "thermal_state": floor.get("thermal_state"),
        "power": floor.get("power"),
    }
    for field, value in expected.items():
        if isinstance(value, str) and device.get(field) != value:
            raise ReleaseValidationError(
                f"{fragment_path}: physical device {field} does not match floor-device contract"
            )
    allowed_models = floor.get("allowed_models")
    if not isinstance(allowed_models, list) or not allowed_models or not all(
        isinstance(model, str) and model for model in allowed_models
    ):
        raise ReleaseValidationError(
            "manifest floor_device.allowed_models must be a non-empty string list"
        )
    if device.get("model") not in allowed_models:
        raise ReleaseValidationError(
            f"{fragment_path}: physical device model is outside the floor-device class"
        )
    minimum_os = floor.get("minimum_os")
    if isinstance(minimum_os, str):
        try:
            actual_parts = tuple(
                int(part) for part in str(device["os_version"]).split(".")
            )
            minimum_parts = tuple(int(part) for part in minimum_os.split("."))
        except ValueError as error:
            raise ReleaseValidationError(
                f"{fragment_path}: device OS provenance is not numeric"
            ) from error
        family_width = min(2, len(minimum_parts))
        if actual_parts[:family_width] != minimum_parts[:family_width]:
            raise ReleaseValidationError(
                f"{fragment_path}: physical device OS is not in the floor release family"
            )


def _artifact_records(fragment: Mapping[str, Any], fragment_path: Path) -> list[JsonObject]:
    raw_artifacts = fragment.get("artifacts")
    if not isinstance(raw_artifacts, list) or not raw_artifacts:
        raise ReleaseValidationError(f"{fragment_path}: artifacts must be non-empty")
    records = []
    for raw in raw_artifacts:
        if not isinstance(raw, Mapping):
            raise ReleaseValidationError(f"{fragment_path}: artifact must be an object")
        path_value = _require_string(raw.get("path"), "artifact path")
        expected = _require_digest(raw.get("sha256"), "artifact sha256")
        artifact_path = Path(path_value)
        if not artifact_path.is_absolute():
            artifact_path = fragment_path.parent / artifact_path
        actual, artifact_bytes = sha256_artifact(artifact_path)
        if actual != expected:
            raise ReleaseValidationError(f"artifact SHA-256 mismatch: {path_value}")
        artifact_name = _require_string(raw.get("name", artifact_path.name), "artifact name")
        if Path(artifact_name).name != artifact_name or any(
            ord(character) < 32 for character in artifact_name
        ):
            raise ReleaseValidationError("artifact name must be a sanitized basename")
        records.append(
            {
                "name": artifact_name,
                "sha256": actual,
                "bytes": artifact_bytes,
            }
        )
    return records


def aggregate_fragments(
    manifest: Mapping[str, Any],
    catalog: Mapping[str, Any],
    specs: Mapping[str, MeasurementSpec],
    manifest_digest: str,
    catalog_digest: str,
    fragment_paths: Iterable[str | Path],
) -> JsonObject:
    build_fields = _required_provenance_fields(manifest, "build", _BUILD_FIELDS)
    device_fields = _required_provenance_fields(manifest, "device", _DEVICE_FIELDS)
    collected: dict[str, list[float]] = {measurement_id: [] for measurement_id in specs}
    physical: dict[str, bool] = {}
    builds: list[JsonObject] = []
    devices: list[JsonObject] = []
    artifacts: dict[tuple[str, str], JsonObject] = {}
    execution_plan_digest: str | None = None
    interleaving_seed: int | None = None
    fragment_count = 0
    for raw_path in fragment_paths:
        fragment_path = Path(raw_path).resolve()
        fragment = load_json_object(fragment_path, "observation fragment")
        fragment_count += 1
        raw_manifest_digest = fragment.get(
            "profiler_manifest_sha256", fragment.get("manifest_sha256")
        )
        if _require_digest(raw_manifest_digest, "fragment profiler_manifest_sha256") != manifest_digest:
            raise ReleaseValidationError(f"{fragment_path}: manifest SHA-256 does not match")
        raw_catalog_digest = fragment.get(
            "budget_catalog_sha256", fragment.get("catalog_sha256")
        )
        if _require_digest(raw_catalog_digest, "fragment budget_catalog_sha256") != catalog_digest:
            raise ReleaseValidationError(f"{fragment_path}: catalog SHA-256 does not match")
        fragment_plan_digest = _require_digest(
            fragment.get("execution_plan_sha256"), "fragment execution_plan_sha256"
        )
        raw_seed = fragment.get("interleaving_seed")
        if not isinstance(raw_seed, int) or isinstance(raw_seed, bool):
            raise ReleaseValidationError(f"{fragment_path}: interleaving_seed must be an integer")
        if execution_plan_digest is None:
            execution_plan_digest = fragment_plan_digest
            interleaving_seed = raw_seed
        elif execution_plan_digest != fragment_plan_digest or interleaving_seed != raw_seed:
            raise ReleaseValidationError("fragments do not share execution-plan provenance")
        expected_plan_digest = generate_plan(
            manifest, manifest_digest, catalog_digest, raw_seed
        )["execution_plan_sha256"]
        if fragment_plan_digest != expected_plan_digest:
            raise ReleaseValidationError(
                f"{fragment_path}: execution plan SHA-256 does not match seed and manifest"
            )
        build = _sanitize_provenance(fragment.get("build"), build_fields, "build")
        if build.get("configuration") != "release":
            raise ReleaseValidationError(f"{fragment_path}: build configuration must be release")
        if build.get("candidate") == build.get("baseline"):
            raise ReleaseValidationError(f"{fragment_path}: candidate and baseline must differ")
        builds.append(build)
        device = _sanitize_provenance(fragment.get("device"), device_fields, "device")
        _validate_floor_device(manifest, device, fragment_path)
        devices.append(device)
        fragment_artifacts = _artifact_records(fragment, fragment_path)
        fragment_artifact_names = {str(artifact["name"]) for artifact in fragment_artifacts}
        for artifact in fragment_artifacts:
            artifacts[(str(artifact["name"]), str(artifact["sha256"]))] = artifact
        observations = fragment.get("observations")
        if not isinstance(observations, list) or not observations:
            raise ReleaseValidationError(f"{fragment_path}: observations must be non-empty")
        for observation in observations:
            if not isinstance(observation, Mapping):
                raise ReleaseValidationError("observation must be an object")
            measurement_id = _require_string(observation.get("id"), "observation id")
            spec = specs.get(measurement_id)
            if spec is None:
                raise ReleaseValidationError(f"unknown observation: {measurement_id}")
            missing_artifacts = sorted(
                set(spec.required_artifacts) - fragment_artifact_names
            )
            if missing_artifacts:
                raise ReleaseValidationError(
                    f"{measurement_id} lacks required raw artifacts: {missing_artifacts}"
                )
            observed_physical = device.get("physical") is True
            if spec.physical_device_required and not observed_physical:
                raise ReleaseValidationError(f"{measurement_id} requires physical-device evidence")
            prior = physical.setdefault(measurement_id, observed_physical)
            if prior != observed_physical:
                raise ReleaseValidationError(f"{measurement_id} mixes device provenance")
            if spec.sample_shape == "pair":
                pairs = observation.get("pairs")
                if not isinstance(pairs, list) or not pairs:
                    raise ReleaseValidationError(f"{measurement_id} requires pairs")
                for pair in pairs:
                    if not isinstance(pair, Mapping):
                        raise ReleaseValidationError(f"{measurement_id} pair must be an object")
                    baseline = _finite_number(pair.get("baseline"), f"{measurement_id} baseline")
                    candidate = _finite_number(pair.get("candidate"), f"{measurement_id} candidate")
                    collected[measurement_id].append(candidate - baseline)
            else:
                values = observation.get("values")
                if values is None and "value" in observation:
                    values = [observation["value"]]
                if not isinstance(values, list) or not values:
                    raise ReleaseValidationError(f"{measurement_id} requires scalar values")
                collected[measurement_id].extend(
                    _finite_number(value, f"{measurement_id} value") for value in values
                )
    if fragment_count == 0:
        raise ReleaseValidationError("at least one observation fragment is required")
    first_build = builds[0]
    if any(build != first_build for build in builds[1:]):
        raise ReleaseValidationError("fragments do not share exact build provenance")
    measurements = []
    for measurement_id in sorted(specs):
        spec = specs[measurement_id]
        values = collected[measurement_id]
        if len(values) < spec.minimum_samples:
            raise ReleaseValidationError(
                f"{measurement_id} has {len(values)} samples; requires {spec.minimum_samples}"
            )
        measurements.append(
            {
                "id": measurement_id,
                "value": aggregate_statistic(spec.statistic, values, measurement_id),
                "unit": spec.unit,
                "statistic": spec.statistic,
                "samples": len(values),
                "method": spec.method,
                "build_mode": "release",
                "physical_device": physical.get(measurement_id, False),
            }
        )
    unique_devices = sorted(
        {_canonical_sha256(device): device for device in devices}.values(),
        key=_canonical_sha256,
    )
    return {
        "catalog_version": str(catalog["schema_version"]),
        "profile": EXPECTED_PROFILE,
        "candidate": first_build["candidate"],
        "baseline": first_build["baseline"],
        "measurements": measurements,
        "release_provenance": {
            "manifest_sha256": manifest_digest,
            "profiler_manifest_sha256": manifest_digest,
            "catalog_sha256": catalog_digest,
            "budget_catalog_sha256": catalog_digest,
            "execution_plan_sha256": execution_plan_digest,
            "interleaving_seed": interleaving_seed,
            "candidate_identity": first_build["candidate"],
            "compiled_out_baseline_identity": first_build["baseline"],
            "build_mode": first_build["configuration"],
            "build": first_build,
            "devices": unique_devices,
            "artifacts": sorted(
                (
                    {"sha256": item["sha256"], "bytes": item["bytes"]}
                    for item in artifacts.values()
                ),
                key=lambda item: (item["sha256"], item["bytes"]),
            ),
            "observation_fragments": fragment_count,
        },
    }


def physical_preflight(
    device_id: str,
    development_team: str,
    project: str | Path,
    schemes: Sequence[str],
    floor_device: Mapping[str, Any],
) -> JsonObject:
    """Verify a connected physical target and Release buildability.

    Identifiers are used only as subprocess arguments and are never returned.
    No benchmark command is run and no observation is synthesized.
    """
    _require_string(device_id, "device ID")
    _require_string(development_team, "development team")
    project_path = Path(project).resolve()
    if not project_path.exists():
        raise ReleaseValidationError(f"Xcode project does not exist: {project_path}")
    with tempfile.TemporaryDirectory(prefix="chill-device-preflight-") as temporary:
        listing = Path(temporary) / "devices.json"
        command = [
            "xcrun",
            "devicectl",
            "list",
            "devices",
            "--json-output",
            str(listing),
        ]
        completed = subprocess.run(command, text=True, capture_output=True, check=False)
        if completed.returncode != 0:
            raise ReleaseValidationError("devicectl could not enumerate physical devices")
        listing_text = listing.read_text(encoding="utf-8") if listing.exists() else completed.stdout
        try:
            listing_value = json.loads(listing_text)
        except json.JSONDecodeError as error:
            raise ReleaseValidationError("devicectl returned malformed device JSON") from error
        device_record = _find_device_record(listing_value, device_id)
        if device_record is None:
            raise ReleaseValidationError("requested physical device is not connected")
        record_text = json.dumps(device_record, sort_keys=True).lower()
        if any(marker in record_text for marker in ('"unavailable"', '"disconnected"', '"unpaired"')):
            raise ReleaseValidationError("requested physical device is unavailable")
        if "simulator" in record_text:
            raise ReleaseValidationError("release preflight requires a physical device")
        device_os = _record_string(
            device_record, ("osVersion", "productVersion", "operatingSystemVersion")
        )
        device_model = _record_string(
            device_record, ("marketingName", "productType", "modelName", "model")
        )
        if device_os is None or device_model is None:
            raise ReleaseValidationError("devicectl omitted device model or OS provenance")
        _validate_floor_device(
            {"floor_device": floor_device},
            {
                "class": floor_device.get("class"),
                "model": device_model,
                "os_version": device_os,
                "physical": True,
                "thermal_state": floor_device.get("thermal_state"),
                "power": floor_device.get("power"),
            },
            Path("devicectl"),
        )
    checked_schemes = []
    with tempfile.TemporaryDirectory(prefix="chill-xcode-preflight-") as temporary:
        for scheme in schemes:
            _require_string(scheme, "Xcode scheme")
            settings_command = [
                "xcodebuild",
                "-project",
                str(project_path),
                "-scheme",
                scheme,
                "-configuration",
                "Release",
                "-destination",
                f"id={device_id}",
                f"DEVELOPMENT_TEAM={development_team}",
                "-showBuildSettings",
            ]
            completed = subprocess.run(
                settings_command, text=True, capture_output=True, check=False
            )
            if completed.returncode != 0:
                raise ReleaseValidationError(
                    f"Release preflight failed for Xcode scheme {scheme}"
                )
            variant = "Baseline" if "Baseline" in scheme else "Candidate"
            target = f"ChillPhysical{variant}UITests"
            result_path = Path(temporary) / f"{variant}.xcresult"
            test_command = [
                "xcodebuild",
                "-project",
                str(project_path),
                "-scheme",
                scheme,
                "-configuration",
                "Release",
                "-destination",
                f"id={device_id}",
                f"DEVELOPMENT_TEAM={development_team}",
                "-resultBundlePath",
                str(result_path),
                "-only-testing:"
                f"{target}/ChillPhysicalValidationUITests/testReleaseEnvironmentPreflight",
                "CHILL_RELEASE_WORKLOAD_GATE=1",
                "test",
            ]
            environment = os.environ.copy()
            environment["CHILL_RELEASE_WORKLOAD_GATE"] = "1"
            completed = subprocess.run(
                test_command,
                text=True,
                capture_output=True,
                check=False,
                env=environment,
            )
            if completed.returncode != 0:
                raise ReleaseValidationError(
                    f"physical environment preflight failed for Xcode scheme {scheme}"
                )
            _require_executed_test(completed, f"environment preflight for {scheme}")
            checked_schemes.append(scheme)
    return {
        "physical_device_connected": True,
        "release_build_settings_verified": checked_schemes,
        "floor_os_verified": True,
        "thermal_nominal_verified_by_xctest": True,
        "unplugged_power_verified_by_xctest": True,
        "measurements_collected": 0,
    }


def _find_device_record(value: object, device_id: str) -> Mapping[str, Any] | None:
    if isinstance(value, Mapping):
        if any(item == device_id for item in value.values() if isinstance(item, str)):
            return value
        for child in value.values():
            found = _find_device_record(child, device_id)
            if found is not None:
                return found
    elif isinstance(value, list):
        for child in value:
            found = _find_device_record(child, device_id)
            if found is not None:
                return found
    return None


def _record_string(
    value: object, keys: Sequence[str]
) -> str | None:
    if isinstance(value, Mapping):
        for key in keys:
            candidate = value.get(key)
            if isinstance(candidate, str) and candidate:
                return candidate
        for child in value.values():
            candidate = _record_string(child, keys)
            if candidate is not None:
                return candidate
    elif isinstance(value, list):
        for child in value:
            candidate = _record_string(child, keys)
            if candidate is not None:
                return candidate
    return None


def _require_executed_test(
    completed: subprocess.CompletedProcess[str], label: str
) -> None:
    output = f"{completed.stdout or ''}\n{completed.stderr or ''}".lower()
    if "skipped" in output or "executed 0 tests" in output or "0 tests executed" in output:
        raise ReleaseValidationError(f"{label} did not execute its selected XCTest")


def execute_xctest_plan(
    manifest: Mapping[str, Any],
    plan: Mapping[str, Any],
    device_id: str,
    development_team: str,
    project: str | Path,
    evidence_directory: str | Path,
) -> JsonObject:
    """Run every XCTest-backed plan task in its exact scheduled order."""
    scenarios = list(manifest.get("scenarios", []))
    supplemental = manifest.get("supplemental_scenarios", [])
    if isinstance(supplemental, list):
        scenarios.extend(supplemental)
    scenario_by_id = {
        str(scenario["id"]): scenario
        for scenario in scenarios
        if isinstance(scenario, Mapping)
    }
    tasks = plan.get("tasks")
    if not isinstance(tasks, list):
        raise ReleaseValidationError("execution plan tasks must be a list")
    evidence_root = Path(evidence_directory).resolve()
    run_root = evidence_root / f"release-{str(plan['execution_plan_sha256'])[:16]}"
    if run_root.exists():
        raise ReleaseValidationError("raw evidence directory already exists for this plan")
    run_root.mkdir(parents=True)
    executed = 0
    pending_collectors: set[str] = set()
    project_path = Path(project).resolve()
    for task in tasks:
        if not isinstance(task, Mapping):
            raise ReleaseValidationError("execution plan task must be an object")
        scenario = scenario_by_id.get(str(task.get("scenario")))
        if scenario is None:
            raise ReleaseValidationError("execution plan names an unknown scenario")
        collector = _require_string(scenario.get("collector"), "scenario collector")
        method = scenario.get("xctest")
        if not isinstance(method, str) or not method:
            pending_collectors.add(collector)
            continue
        variant = str(task.get("variant"))
        if variant not in {"baseline", "candidate"}:
            raise ReleaseValidationError("execution plan task has invalid variant")
        title = "Baseline" if variant == "baseline" else "Candidate"
        scheme = f"ChillPhysical{title}Validation"
        target = f"ChillPhysical{title}UITests"
        sequence = task.get("sequence")
        if not isinstance(sequence, int) or isinstance(sequence, bool):
            raise ReleaseValidationError("execution plan task requires sequence")
        result_path = run_root / (
            f"{sequence:04d}-{scenario['id']}-{task['run']}-{variant}.xcresult"
        )
        command = [
            "xcodebuild",
            "-project",
            str(project_path),
            "-scheme",
            scheme,
            "-configuration",
            "Release",
            "-destination",
            f"id={device_id}",
            f"DEVELOPMENT_TEAM={development_team}",
            "-resultBundlePath",
            str(result_path),
            "-only-testing:"
            f"{target}/ChillPhysicalValidationUITests/{method}",
        ]
        environment = os.environ.copy()
        environment["CHILL_RELEASE_WORKLOAD_GATE"] = "1"
        build_settings = ["CHILL_RELEASE_WORKLOAD_GATE=1"]
        duration = scenario.get("duration_minutes")
        if isinstance(duration, (int, float)) and not isinstance(duration, bool):
            duration_seconds = str(int(float(duration) * 60))
            environment["CHILL_RELEASE_DURATION_SECONDS"] = duration_seconds
            build_settings.append(
                f"CHILL_RELEASE_DURATION_SECONDS={duration_seconds}"
            )
        if scenario.get("id") == "privacy_canaries":
            canary = (
                f"CHILL-CANARY-{str(plan['execution_plan_sha256'])[:12]}-{sequence:04d}"
            )
            environment["CHILL_PRIVACY_CANARY"] = canary
            build_settings.append(f"CHILL_PRIVACY_CANARY={canary}")
        command.extend((*build_settings, "test"))
        completed = subprocess.run(
            command,
            text=True,
            capture_output=True,
            check=False,
            env=environment,
        )
        if completed.returncode != 0:
            raise ReleaseValidationError(
                f"XCTest plan task {sequence} failed ({scenario['id']} {variant})"
            )
        _require_executed_test(completed, f"XCTest plan task {sequence}")
        executed += 1
        # Even pure XCTest collectors still require parsing into authenticated
        # observation fragments, which this runner intentionally does not fake.
        pending_collectors.add(collector)
    return {
        "status": "incomplete; raw XCTest evidence collected, measurement extraction pending",
        "xctest_tasks_executed": executed,
        "measurements_collected": 0,
        "pending_collectors": sorted(pending_collectors),
    }


def manifest_xcode_configuration(manifest: Mapping[str, Any]) -> tuple[str, tuple[str, ...]]:
    project = "validation/apple/physical/ChillPhysicalValidation.xcodeproj"
    schemes = (
        "ChillPhysicalBaselineValidation",
        "ChillPhysicalCandidateValidation",
    )
    scenarios = list(manifest.get("scenarios", []))
    supplemental = manifest.get("supplemental_scenarios", [])
    if isinstance(supplemental, list):
        scenarios.extend(supplemental)
    for scenario in scenarios:
        if not isinstance(scenario, Mapping) or "xctest" not in scenario:
            continue
        xctest = scenario["xctest"]
        # A string is an XCTest method name, not an Xcode scheme. Only the
        # explicit object form may override projects or add schemes.
        if isinstance(xctest, Mapping):
            if isinstance(xctest.get("project"), str):
                project = str(xctest["project"])
            raw_schemes = xctest.get("schemes")
            if isinstance(raw_schemes, list) and all(isinstance(item, str) for item in raw_schemes):
                schemes = tuple(dict.fromkeys((*schemes, *raw_schemes)))
            elif isinstance(xctest.get("scheme"), str):
                schemes = tuple(dict.fromkeys((*schemes, str(xctest["scheme"]))))
    return project, schemes
