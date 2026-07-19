from __future__ import annotations

from copy import deepcopy
from pathlib import Path
import unittest

from contracts.budgets.v1 import (
    BudgetCatalogError,
    BudgetReportError,
    evaluate_report,
    load_catalog,
    resolve_budgets,
    validate_catalog,
)


ROOT = Path(__file__).resolve().parents[2]
CATALOG_PATH = ROOT / "budgets" / "sdk" / "v1" / "budgets.json"


class BudgetCatalogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.catalog = load_catalog(CATALOG_PATH)

    def passing_report(self, profile: str) -> dict[str, object]:
        methods = self.catalog["methods"]
        measurements: list[dict[str, object]] = []
        for budget in resolve_budgets(self.catalog, profile):
            method = methods[budget.method]
            measurements.append(
                {
                    "id": budget.id,
                    "value": budget.threshold,
                    "unit": budget.unit,
                    "statistic": budget.statistic,
                    "samples": budget.minimum_samples,
                    "method": budget.method,
                    "build_mode": "release",
                    "physical_device": method["physical_device"],
                }
            )
        return {
            "catalog_version": self.catalog["schema_version"],
            "profile": profile,
            "candidate": "candidate-commit",
            "baseline": "compiled-out-control",
            "measurements": measurements,
        }

    def measurement(
        self, report: dict[str, object], budget_id: str
    ) -> dict[str, object]:
        measurements = report["measurements"]
        assert isinstance(measurements, list)
        for value in measurements:
            assert isinstance(value, dict)
            if value["id"] == budget_id:
                return value
        raise AssertionError(f"missing test measurement {budget_id}")

    def test_catalog_covers_every_supported_profile_and_release_dimension(self) -> None:
        self.assertEqual(
            set(self.catalog["profiles"]),
            {
                "apple.core",
                "apple.replay",
                "android.core",
                "android.replay",
                "web.core",
                "web.replay",
                "server.core",
            },
        )
        required = {
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
        for profile_id, profile in self.catalog["profiles"].items():
            with self.subTest(profile=profile_id):
                budgets = resolve_budgets(self.catalog, profile_id)
                categories = {budget.category for budget in budgets}
                self.assertTrue(required <= categories)
                self.assertTrue(all(budget.release_blocking for budget in budgets))
                if profile["feature_set"] == "replay":
                    self.assertIn("replay", categories)

    def test_swift_first_thresholds_are_small_and_concrete(self) -> None:
        budgets = {
            budget.id: budget
            for budget in resolve_budgets(self.catalog, "apple.core")
        }

        self.assertEqual(budgets["package.download_delta_bytes"].threshold, 2 * 1024**2)
        self.assertEqual(budgets["startup.p95_delta_ms"].threshold, 10)
        self.assertEqual(budgets["hot_path.disabled_allocations_per_call"].threshold, 0)
        self.assertEqual(budgets["hot_path.capture_sync_p99_us"].threshold, 250)
        self.assertEqual(budgets["memory.steady_delta_bytes"].threshold, 4 * 1024**2)
        self.assertEqual(budgets["cpu.energy_delta_percent"].threshold, 3)
        self.assertEqual(budgets["network.compressed_bytes_per_record"].threshold, 750)
        self.assertEqual(budgets["reliability.high_value_drops_count"].threshold, 0)

    def test_replay_profiles_inherit_core_and_add_stricter_replay_gates(self) -> None:
        core = {
            budget.id: budget
            for budget in resolve_budgets(self.catalog, "apple.core")
        }
        replay = {
            budget.id: budget
            for budget in resolve_budgets(self.catalog, "apple.replay")
        }

        self.assertTrue(core.keys() < replay.keys())
        self.assertEqual(
            replay["package.download_delta_bytes"].threshold,
            core["package.download_delta_bytes"].threshold,
        )
        self.assertEqual(replay["replay.frame_capture_p95_ms"].threshold, 1)
        self.assertEqual(replay["replay.unredacted_frames_count"].threshold, 0)
        self.assertEqual(replay["replay.raw_sensitive_buffer_bytes"].threshold, 0)

    def test_reports_at_the_threshold_pass_for_every_profile(self) -> None:
        for profile in self.catalog["profiles"]:
            with self.subTest(profile=profile):
                evaluation = evaluate_report(
                    self.catalog, self.passing_report(profile)
                )
                self.assertTrue(evaluation.passed)
                self.assertTrue(all(result.passed for result in evaluation.results))

    def test_threshold_breach_and_missing_measurement_block_release(self) -> None:
        report = self.passing_report("apple.core")
        startup = self.measurement(report, "startup.p95_delta_ms")
        startup["value"] = 10.001

        evaluation = evaluate_report(self.catalog, report)

        self.assertFalse(evaluation.passed)
        failed = [result for result in evaluation.results if not result.passed]
        self.assertEqual([result.id for result in failed], ["startup.p95_delta_ms"])
        self.assertEqual(failed[0].reason, "threshold exceeded")

        missing_report = self.passing_report("web.core")
        measurements = missing_report["measurements"]
        assert isinstance(measurements, list)
        measurements[:] = [
            measurement
            for measurement in measurements
            if measurement["id"] != "privacy.denied_outbound_bytes"
        ]
        missing = evaluate_report(self.catalog, missing_report)
        missing_results = [result for result in missing.results if not result.passed]
        self.assertEqual(
            [result.id for result in missing_results],
            ["privacy.denied_outbound_bytes"],
        )
        self.assertEqual(missing_results[0].reason, "required measurement is missing")

    def test_report_provenance_cannot_be_weakened(self) -> None:
        mutations = {
            "wrong version": lambda report: report.update(
                catalog_version="2.0.0"
            ),
            "same baseline": lambda report: report.update(
                baseline="candidate-commit"
            ),
            "debug build": lambda report: self.measurement(
                report, "startup.p95_delta_ms"
            ).update(build_mode="debug"),
            "wrong method": lambda report: self.measurement(
                report, "startup.p95_delta_ms"
            ).update(method="apple.xctest_micro"),
            "simulated launch": lambda report: self.measurement(
                report, "startup.p95_delta_ms"
            ).update(physical_device=False),
            "too few samples": lambda report: self.measurement(
                report, "startup.p95_delta_ms"
            ).update(samples=29),
            "nonboolean device provenance": lambda report: self.measurement(
                report, "startup.p95_delta_ms"
            ).update(physical_device="yes"),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name):
                report = self.passing_report("apple.core")
                mutate(report)
                with self.assertRaises(BudgetReportError):
                    evaluate_report(self.catalog, report)

        unknown_profile = self.passing_report("apple.core")
        unknown_profile["profile"] = "apple.future"
        with self.assertRaisesRegex(BudgetReportError, "unknown profile"):
            evaluate_report(self.catalog, unknown_profile)

    def test_unknown_and_duplicate_measurements_are_rejected(self) -> None:
        unknown = self.passing_report("server.core")
        measurements = unknown["measurements"]
        assert isinstance(measurements, list)
        measurements.append(
            {
                "id": "cpu.made_up",
                "value": 0,
                "unit": "percent",
                "statistic": "total",
                "samples": 1,
                "method": "server.process_ab",
                "build_mode": "release",
                "physical_device": False,
            }
        )
        with self.assertRaisesRegex(BudgetReportError, "unknown measurement"):
            evaluate_report(self.catalog, unknown)

        duplicate = self.passing_report("server.core")
        duplicate_measurements = duplicate["measurements"]
        assert isinstance(duplicate_measurements, list)
        duplicate_measurements.append(deepcopy(duplicate_measurements[0]))
        with self.assertRaisesRegex(BudgetReportError, "duplicate measurement"):
            evaluate_report(self.catalog, duplicate)

    def test_catalog_cannot_hide_a_nonblocking_or_inheritance_escape_hatch(self) -> None:
        nonblocking = deepcopy(self.catalog)
        nonblocking["budgets"][0]["release_blocking"] = False
        with self.assertRaisesRegex(BudgetCatalogError, "release blocking"):
            validate_catalog(nonblocking)

        cyclic = deepcopy(self.catalog)
        cyclic["profiles"]["apple.core"]["extends"] = "apple.replay"
        with self.assertRaisesRegex(BudgetCatalogError, "cycle"):
            validate_catalog(cyclic)


if __name__ == "__main__":
    unittest.main()
