from __future__ import annotations

from copy import deepcopy
from pathlib import Path
import unittest

from contracts.conformance.v1 import load_suite as load_v1, suite_digest as digest_v1
from contracts.conformance.v2 import ConformanceReportError, evaluate_implementation_report, load_suite, run_suite, suite_digest

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "conformance/v2/manifest.json"
V1 = ROOT / "conformance/v1/manifest.json"

class PortableV2Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest, cls.scenarios = load_suite(MANIFEST)
        cls.digest = suite_digest(cls.manifest, cls.scenarios)
        cls.by_id = {s["id"]: s for s in cls.scenarios}
        cls.v1_manifest, cls.v1_scenarios = load_v1(V1)

    def v1_report(self, profile):
        ids = list(self.v1_manifest["profiles"][profile].get("scenarios", []))
        parent = self.v1_manifest["profiles"][profile].get("extends")
        if parent: ids = list(self.v1_manifest["profiles"][parent]["scenarios"]) + ids
        data = {s["id"]: s for s in self.v1_scenarios}
        return {"suite_version":"1.0.0", "suite_digest":digest_v1(self.v1_manifest,self.v1_scenarios), "profile":profile, "implementation":{"platform":"server" if profile == "server.core" else "apple", "adapter":"test", "sdk_version":"0", "layer":"adapter"}, "results":[{"scenario_id":sid,"status":"passed","output":deepcopy(data[sid]["expect"])} for sid in ids]}

    def report(self, profile="portable.rust"):
        config=self.manifest["profiles"][profile]
        return {"suite_version":"2.0.0", "suite_digest":self.digest, "profile":profile, "implementation":{"platform":config["platform"],"adapter":config["adapter"],"sdk_version":"0","layer":"adapter"}, "v1_report":self.v1_report(config["v1_profile"]), "results":[{"scenario_id":sid,"status":"passed","output":deepcopy(self.by_id[sid]["expect"])} for sid in config["scenarios"]]}

    def test_reference_is_deterministic_and_binds_v1_exactly(self):
        self.assertEqual(run_suite(MANIFEST), run_suite(MANIFEST))
        self.assertTrue(run_suite(MANIFEST).passed)
        self.assertEqual(self.manifest["v1_binding"]["suite_digest"], digest_v1(self.v1_manifest,self.v1_scenarios))

    def test_all_runtime_profiles_require_v1_and_portable_cases(self):
        for profile in self.manifest["profiles"]:
            with self.subTest(profile=profile): self.assertTrue(evaluate_implementation_report(MANIFEST,self.report(profile)).passed)

    def test_rejects_skips_duplicates_stale_profile_and_layer(self):
        report=self.report(); report["results"][0]["status"]="skipped"
        with self.assertRaisesRegex(ConformanceReportError,"may not be skipped"): evaluate_implementation_report(MANIFEST,report)
        report=self.report(); report["results"].append(deepcopy(report["results"][0]))
        with self.assertRaisesRegex(ConformanceReportError,"duplicate"): evaluate_implementation_report(MANIFEST,report)
        report=self.report(); report["suite_digest"]="0"*64
        with self.assertRaisesRegex(ConformanceReportError,"different conformance"): evaluate_implementation_report(MANIFEST,report)
        report=self.report(); report["implementation"]["layer"]="unit"
        with self.assertRaisesRegex(ConformanceReportError,"layer is invalid"): evaluate_implementation_report(MANIFEST,report)
        report=self.report(); report["v1_report"]["profile"]="client.core"
        with self.assertRaisesRegex(ConformanceReportError,"does not match"): evaluate_implementation_report(MANIFEST,report)

    def test_canaries_and_portable_semantics_are_pinned(self):
        output={x.scenario_id:x.actual for x in run_suite(MANIFEST).results}
        self.assertFalse(output["ipc.metadata_correlation"]["untrusted"]["accepted"])
        self.assertEqual(output["async.producer_consumer_links"]["link_count"],1)
        self.assertEqual(output["attributes.bounded_default_deny"]["policy"],"default_deny")
        self.assertEqual(output["offline.retry_identity"]["attempts"][0],output["offline.retry_identity"]["attempts"][1])
        report=self.report(); report["results"][-1]["output"]["leak"]="ipc-canary-input"
        failed=evaluate_implementation_report(MANIFEST,report)
        self.assertFalse(failed.passed)
        self.assertIn("forbidden_token:ipc-canary-input",failed.results[-1].reasons)

if __name__ == "__main__": unittest.main()
