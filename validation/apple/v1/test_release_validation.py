from __future__ import annotations

from copy import deepcopy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from contracts.budgets.v1.reference import evaluate_report, load_catalog, resolve_budgets
from validation.apple.v1.release_validation import (
    ReleaseValidationError,
    aggregate_fragments,
    execute_xctest_plan,
    generate_plan,
    measurement_specs,
    physical_preflight,
    sha256_file,
)


ROOT = Path(__file__).resolve().parents[3]
CATALOG_PATH = ROOT / "budgets/sdk/v1/budgets.json"


def fixture_manifest(catalog: dict[str, object]) -> dict[str, object]:
    grouped: dict[str, list[dict[str, object]]] = {}
    physical: dict[str, bool] = {}
    for budget in resolve_budgets(catalog, "apple.replay"):
        paired = budget.statistic.endswith("paired_delta") or budget.statistic == "max_paired_delta"
        grouped.setdefault(budget.method, []).append(
            {
                "id": budget.id,
                "unit": budget.unit,
                "statistic": budget.statistic,
                "minimum_samples": budget.minimum_samples,
                "sample_shape": "pair" if paired else "scalar",
            }
        )
        physical[budget.method] = budget.physical_device_required
    scenarios = []
    for method, measurements in grouped.items():
        scenarios.append(
            {
                "id": method.replace(".", "_"),
                "method": method,
                "collector": "unit-test collector",
                "physical_device_required": physical[method],
                "paired": any(item["sample_shape"] == "pair" for item in measurements),
                "minimum_runs": max(int(item["minimum_samples"]) for item in measurements),
                "workload": "deterministic test workload",
                "measurements": measurements,
            }
        )
    return {
        "schema_version": "1.0.0",
        "profile": "apple.replay",
        "budget_catalog": "budgets/sdk/v1/budgets.json",
        "floor_device": {
            "class": "oldest supported physical iPhone",
            "allowed_models": ["fixture model", "iPhone fixture"],
            "minimum_os": "17.0",
            "thermal_state": "nominal",
            "power": "unplugged",
        },
        "provenance_requirements": {
            "build": {"required": [
                "candidate", "baseline", "revision", "configuration",
                "xcode_version", "swift_version",
            ]},
            "device": {"required": [
                "class", "model", "os_version", "physical", "thermal_state", "power",
            ]},
        },
        "interleaving": {"seeded": True, "unit": "pair"},
        "scenarios": scenarios,
    }


class ReleaseValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.catalog = load_catalog(CATALOG_PATH)
        self.manifest = fixture_manifest(self.catalog)
        self.specs = measurement_specs(self.manifest, self.catalog)

    def test_manifest_defines_exactly_40_catalog_measurements(self) -> None:
        self.assertEqual(len(self.specs), 40)
        broken = deepcopy(self.manifest)
        broken["scenarios"][0]["measurements"].pop()  # type: ignore[index]
        with self.assertRaisesRegex(ReleaseValidationError, "exactly the 40"):
            measurement_specs(broken, self.catalog)

    def test_plan_is_deterministic_and_interleaves_every_pair(self) -> None:
        first = generate_plan(self.manifest, "a" * 64, "b" * 64, 913)
        second = generate_plan(self.manifest, "a" * 64, "b" * 64, 913)
        third = generate_plan(self.manifest, "a" * 64, "b" * 64, 914)
        self.assertEqual(first, second)
        self.assertNotEqual(first["plan_id"], third["plan_id"])
        tasks = first["tasks"]
        self.assertIsInstance(tasks, list)
        paired_scenarios = {
            scenario["id"] for scenario in self.manifest["scenarios"]  # type: ignore[index]
            if scenario["paired"]
        }
        for scenario_id in paired_scenarios:
            scenario_tasks = [task for task in tasks if task["scenario"] == scenario_id]
            for index in range(0, len(scenario_tasks), 2):
                pair = scenario_tasks[index:index + 2]
                self.assertEqual({item["variant"] for item in pair}, {"baseline", "candidate"})
                self.assertEqual(pair[0]["pair"], pair[1]["pair"])

    def test_aggregation_verifies_hashes_and_emits_evaluator_report(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_name:
            temporary = Path(temporary_name)
            artifact = temporary / "trace.data"
            artifact.write_bytes(b"immutable raw trace")
            artifact_hash = sha256_file(artifact)
            manifest_hash = hashlib.sha256(b"manifest fixture").hexdigest()
            catalog_hash = sha256_file(CATALOG_PATH)
            plan_hash = generate_plan(
                self.manifest, manifest_hash, catalog_hash, 913
            )["execution_plan_sha256"]
            build = {
                "candidate": "candidate-deadbeef",
                "baseline": "baseline-deadbeef",
                "revision": "deadbeef",
                "configuration": "release",
                "xcode_version": "Xcode fixture",
                "swift_version": "Swift fixture",
            }
            base_device = {
                "class": "oldest supported physical iPhone",
                "model": "fixture model",
                "os_version": "17.0",
                "thermal_state": "nominal",
                "power": "unplugged",
            }
            observations: dict[bool, list[dict[str, object]]] = {True: [], False: []}
            for measurement_id, spec in self.specs.items():
                count = spec.minimum_samples
                if spec.sample_shape == "pair":
                    observation = {
                        "id": measurement_id,
                        "pairs": [{"baseline": 0, "candidate": 0} for _ in range(count)],
                    }
                else:
                    observation = {"id": measurement_id, "values": [0] * count}
                observations[spec.physical_device_required].append(observation)
            fragments = []
            for is_physical, entries in observations.items():
                fragment = {
                    "manifest_sha256": manifest_hash,
                    "catalog_sha256": catalog_hash,
                    "execution_plan_sha256": plan_hash,
                    "interleaving_seed": 913,
                    "build": build,
                    "device": {**base_device, "physical": is_physical},
                    "artifacts": [{
                        "name": "trace",
                        "path": artifact.name,
                        "sha256": artifact_hash,
                    }],
                    "observations": entries,
                }
                path = temporary / f"fragment-{is_physical}.json"
                path.write_text(json.dumps(fragment), encoding="utf-8")
                fragments.append(path)
            report = aggregate_fragments(
                self.manifest,
                self.catalog,
                self.specs,
                manifest_hash,
                catalog_hash,
                fragments,
            )
            self.assertEqual(len(report["measurements"]), 40)
            self.assertEqual(report["candidate"], build["candidate"])
            self.assertNotIn(str(temporary), json.dumps(report))
            self.assertNotIn("trace.data", json.dumps(report))
            evaluation = evaluate_report(self.catalog, report)
            self.assertEqual(len(evaluation.results), 40)

            physical_fragment = fragments[0]
            original_physical = json.loads(
                physical_fragment.read_text(encoding="utf-8")
            )
            for field, bad_value, error_pattern in (
                ("os_version", "18.0", "floor release family"),
                ("thermal_state", "serious", "floor-device contract"),
                ("power", "plugged", "floor-device contract"),
            ):
                altered = deepcopy(original_physical)
                altered["device"][field] = bad_value
                physical_fragment.write_text(json.dumps(altered), encoding="utf-8")
                with self.assertRaisesRegex(ReleaseValidationError, error_pattern):
                    aggregate_fragments(
                        self.manifest,
                        self.catalog,
                        self.specs,
                        manifest_hash,
                        catalog_hash,
                        fragments,
                    )
            physical_fragment.write_text(
                json.dumps(original_physical), encoding="utf-8"
            )

            tampered = json.loads(fragments[0].read_text(encoding="utf-8"))
            tampered["catalog_sha256"] = "0" * 64
            fragments[0].write_text(json.dumps(tampered), encoding="utf-8")
            with self.assertRaisesRegex(ReleaseValidationError, "catalog SHA-256"):
                aggregate_fragments(
                    self.manifest,
                    self.catalog,
                    self.specs,
                    manifest_hash,
                    catalog_hash,
                    fragments,
                )

    def test_sensitive_provenance_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_name:
            temporary = Path(temporary_name)
            artifact = temporary / "raw"
            artifact.write_bytes(b"x")
            sensitive_plan_hash = generate_plan(
                self.manifest, "a" * 64, "b" * 64, 7
            )["execution_plan_sha256"]
            fragment = temporary / "fragment.json"
            fragment.write_text(json.dumps({
                "manifest_sha256": "a" * 64,
                "catalog_sha256": "b" * 64,
                "execution_plan_sha256": sensitive_plan_hash,
                "interleaving_seed": 7,
                "build": {
                    "candidate": "c", "baseline": "b", "revision": "r",
                    "configuration": "release", "xcode_version": "x",
                    "swift_version": "s", "development_team": "secret",
                },
                "device": {},
                "artifacts": [{"path": "raw", "sha256": sha256_file(artifact)}],
                "observations": [],
            }), encoding="utf-8")
            with self.assertRaisesRegex(ReleaseValidationError, "sensitive fields"):
                aggregate_fragments(
                    self.manifest, self.catalog, self.specs, "a" * 64, "b" * 64, [fragment]
                )

    @mock.patch("validation.apple.v1.release_validation.subprocess.run")
    def test_execute_preflight_uses_devicectl_and_xcodebuild_without_measurements(
        self, run: mock.Mock
    ) -> None:
        device = "private-device-id"
        team = "private-team-id"
        run.side_effect = [
            subprocess.CompletedProcess(
                [], 0,
                stdout=json.dumps({"result": {"devices": [{
                    "identifier": device,
                    "deviceProperties": {"osVersion": "17.0.3"},
                    "hardwareProperties": {
                        "platform": "iOS", "marketingName": "iPhone fixture"
                    },
                    "connectionProperties": {"pairingState": "paired"},
                }]}}),
                stderr="",
            ),
            subprocess.CompletedProcess([], 0, stdout="settings", stderr=""),
            subprocess.CompletedProcess([], 0, stdout="test passed", stderr=""),
        ]
        with tempfile.TemporaryDirectory() as temporary_name:
            project = Path(temporary_name) / "Fixture.xcodeproj"
            project.mkdir()
            result = physical_preflight(
                device,
                team,
                project,
                ["Fixture"],
                self.manifest["floor_device"],  # type: ignore[arg-type]
            )
        self.assertEqual(result["measurements_collected"], 0)
        self.assertNotIn(device, json.dumps(result))
        self.assertNotIn(team, json.dumps(result))
        commands = [call.args[0] for call in run.call_args_list]
        self.assertIn("devicectl", commands[0])
        self.assertIn("xcodebuild", commands[1])
        self.assertIn("-showBuildSettings", commands[1])
        self.assertIn("testReleaseEnvironmentPreflight", " ".join(commands[2]))
        self.assertIn("CHILL_RELEASE_WORKLOAD_GATE=1", commands[2])
        self.assertEqual(
            run.call_args_list[2].kwargs["env"]["CHILL_RELEASE_WORKLOAD_GATE"], "1"
        )

    @mock.patch("validation.apple.v1.release_validation.subprocess.run")
    def test_execute_runs_xctest_tasks_in_plan_order_and_stays_incomplete(
        self, run: mock.Mock
    ) -> None:
        run.return_value = subprocess.CompletedProcess([], 0, stdout="", stderr="")
        manifest = {
            "scenarios": [
                {
                    "id": "cold_launch",
                    "collector": "xcodebuild_xcresult",
                    "xctest": "testReleaseColdLaunchSingleSample",
                },
                {
                    "id": "power",
                    "collector": "xcode_power_profiler",
                },
            ]
        }
        plan = {
            "execution_plan_sha256": "d" * 64,
            "tasks": [
                {"scenario": "cold_launch", "run": 2, "variant": "candidate", "sequence": 1},
                {"scenario": "power", "run": 1, "variant": "baseline", "sequence": 2},
                {"scenario": "cold_launch", "run": 1, "variant": "baseline", "sequence": 3},
            ],
        }
        with tempfile.TemporaryDirectory() as temporary_name:
            temporary = Path(temporary_name)
            project = temporary / "Fixture.xcodeproj"
            project.mkdir()
            result = execute_xctest_plan(
                manifest, plan, "secret-device", "secret-team", project, temporary / "evidence"
            )
        self.assertEqual(result["status"].split(";")[0], "incomplete")
        self.assertEqual(result["xctest_tasks_executed"], 2)
        self.assertEqual(result["measurements_collected"], 0)
        commands = [call.args[0] for call in run.call_args_list]
        self.assertIn("ChillPhysicalCandidateValidation", commands[0])
        self.assertIn("ChillPhysicalBaselineValidation", commands[1])
        self.assertIn("testReleaseColdLaunchSingleSample", " ".join(commands[0]))
        self.assertIn("CHILL_RELEASE_WORKLOAD_GATE=1", commands[0])
        self.assertNotIn("secret-device", json.dumps(result))
        self.assertNotIn("secret-team", json.dumps(result))


if __name__ == "__main__":
    unittest.main()
