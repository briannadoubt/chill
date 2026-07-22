from __future__ import annotations

from copy import deepcopy
from datetime import datetime, timedelta, timezone
from pathlib import Path
import unittest

from contracts.budgets.v2 import BudgetCatalogError, BudgetReportError, evaluate_report, load_catalog, resolve_budgets, validate_catalog

ROOT = Path(__file__).resolve().parents[2]
CATALOG = ROOT / "budgets/sdk/v2/budgets.json"
NOW = datetime(2026, 7, 21, 12, tzinfo=timezone.utc)


class PortableV2BudgetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): cls.catalog = load_catalog(CATALOG)

    def report(self, profile="rust.core"):
        return {"catalog_version": "2.0.0", "profile": profile, "candidate": "abc", "baseline": "control", "generated_at": NOW.isoformat(), "measurements": [{"id": b.id, "profile": profile, "value": b.threshold, "unit": b.unit, "statistic": b.statistic, "samples": b.minimum_samples, "method": b.method, "build_mode": "release"} for b in resolve_budgets(self.catalog, profile)]}

    def item(self, report, ident): return next(m for m in report["measurements"] if m["id"] == ident)

    def test_catalog_is_separately_versioned_v1_bound_and_covers_six_release_profiles(self):
        self.assertEqual(self.catalog["schema_version"], "2.0.0")
        self.assertEqual(self.catalog["binds_v1"]["schema_version"], "1.0.0")
        self.assertEqual(set(self.catalog["profiles"]), {"rust.core", "javascript.runtime", "electron.host", "electron.renderer", "tauri.host", "tauri.webview"})
        for profile in self.catalog["profiles"]:
            budgets = resolve_budgets(self.catalog, profile)
            self.assertTrue(all(b.threshold >= 0 for b in budgets))
            self.assertTrue(all(self.catalog["profiles"][profile]["release_blocking"] for _ in budgets))

    def test_each_profile_passes_exactly_at_release_thresholds(self):
        for profile in self.catalog["profiles"]:
            with self.subTest(profile=profile): self.assertTrue(evaluate_report(self.catalog, self.report(profile), now=NOW).passed)

    def test_threshold_missing_duplicate_wrong_profile_nonfinite_and_undersampled_are_rejected_or_fail(self):
        exceeded = self.report(); self.item(exceeded, "cpu.sustained_delta_percent")["value"] = 2.01
        result = evaluate_report(self.catalog, exceeded, now=NOW)
        self.assertFalse(result.passed)
        self.assertEqual(next(r.reason for r in result.results if r.id == "cpu.sustained_delta_percent"), "threshold exceeded")
        missing = self.report(); missing["measurements"] = missing["measurements"][1:]
        self.assertFalse(evaluate_report(self.catalog, missing, now=NOW).passed)
        mutations = {
            "duplicate": lambda r: r["measurements"].append(deepcopy(r["measurements"][0])),
            "wrong-profile": lambda r: self.item(r, "cpu.sustained_delta_percent").update(profile="tauri.host"),
            "non-finite": lambda r: self.item(r, "cpu.sustained_delta_percent").update(value=float("nan")),
            "under-sampled": lambda r: self.item(r, "cpu.sustained_delta_percent").update(samples=29),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                report = self.report(); mutate(report)
                with self.assertRaises(BudgetReportError): evaluate_report(self.catalog, report, now=NOW)

    def test_stale_catalog_and_bad_binding_are_rejected(self):
        stale = self.report(); stale["generated_at"] = (NOW - timedelta(hours=169)).isoformat()
        with self.assertRaisesRegex(BudgetReportError, "stale"): evaluate_report(self.catalog, stale, now=NOW)
        bad = deepcopy(self.catalog); bad["binds_v1"]["schema_version"] = "9.0.0"
        with self.assertRaisesRegex(BudgetCatalogError, "bind"): validate_catalog(bad)


if __name__ == "__main__": unittest.main()
