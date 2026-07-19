from __future__ import annotations

from copy import deepcopy
import json
from pathlib import Path
import tempfile
import unittest

from contracts.conformance.v1 import (
    ConformanceReportError,
    ConformanceSuiteError,
    evaluate_implementation_report,
    load_suite,
    run_suite,
    suite_digest,
)


ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = ROOT / "conformance" / "v1" / "manifest.json"


class CrossPlatformSuiteTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest, cls.scenarios = load_suite(MANIFEST_PATH)
        cls.scenarios_by_id = {
            scenario["id"]: scenario for scenario in cls.scenarios
        }
        cls.digest = suite_digest(cls.manifest, cls.scenarios)

    def passing_report(
        self, profile: str, *, layer: str = "adapter"
    ) -> dict[str, object]:
        scenario_ids: list[str] = []
        profile_value = self.manifest["profiles"][profile]
        parent = profile_value.get("extends")
        if parent is not None:
            scenario_ids.extend(self.manifest["profiles"][parent]["scenarios"])
        scenario_ids.extend(profile_value["scenarios"])
        return {
            "suite_version": self.manifest["suite_version"],
            "suite_digest": self.digest,
            "profile": profile,
            "implementation": {
                "platform": "apple" if profile.startswith("client") else "server",
                "adapter": (
                    "swiftui" if profile.startswith("client") else "swift-server"
                ),
                "sdk_version": "0.1.0-test",
                "layer": layer,
            },
            "results": [
                {
                    "scenario_id": scenario_id,
                    "status": "passed",
                    "output": deepcopy(self.scenarios_by_id[scenario_id]["expect"]),
                }
                for scenario_id in scenario_ids
            ],
        }

    def test_reference_suite_is_deterministic_and_covers_every_domain(self) -> None:
        first = run_suite(MANIFEST_PATH)
        second = run_suite(MANIFEST_PATH)

        self.assertTrue(first.passed)
        self.assertEqual(first, second)
        self.assertEqual(len(first.results), 9)
        self.assertEqual(
            {result.domain for result in first.results},
            set(self.manifest["required_domains"]),
        )
        self.assertEqual({result.layer for result in first.results}, {"model"})
        self.assertTrue(
            all(
                result.expected_digest == result.actual_digest
                for result in first.results
            )
        )

    def test_every_fixture_is_required_at_model_adapter_and_wire_layers(self) -> None:
        for scenario in self.scenarios:
            with self.subTest(scenario=scenario["id"]):
                self.assertEqual(
                    set(scenario["layers"]), {"model", "adapter", "wire"}
                )

        for layer in ("model", "adapter", "wire"):
            with self.subTest(layer=layer):
                evaluation = evaluate_implementation_report(
                    MANIFEST_PATH,
                    self.passing_report("client.replay", layer=layer),
                )
                self.assertTrue(evaluation.passed)
                self.assertEqual(
                    {result.layer for result in evaluation.results}, {layer}
                )

    def test_profiles_require_client_automation_and_replay_only_when_enabled(
        self,
    ) -> None:
        core = self.passing_report("client.core")
        replay = self.passing_report("client.replay")
        server = self.passing_report("server.core")

        self.assertEqual(len(core["results"]), 8)
        self.assertEqual(len(replay["results"]), 9)
        self.assertEqual(len(server["results"]), 6)
        self.assertTrue(evaluate_implementation_report(MANIFEST_PATH, core).passed)
        self.assertTrue(evaluate_implementation_report(MANIFEST_PATH, replay).passed)
        self.assertTrue(evaluate_implementation_report(MANIFEST_PATH, server).passed)

    def test_skipped_missing_unknown_and_duplicate_results_are_rejected(self) -> None:
        skipped = self.passing_report("client.core")
        skipped["results"][0]["status"] = "skipped"
        with self.assertRaisesRegex(ConformanceReportError, "may not be skipped"):
            evaluate_implementation_report(MANIFEST_PATH, skipped)

        missing = self.passing_report("client.core")
        missing["results"].pop()
        with self.assertRaisesRegex(ConformanceReportError, "scenario set mismatch"):
            evaluate_implementation_report(MANIFEST_PATH, missing)

        unknown = self.passing_report("client.core")
        unknown["results"].append(
            {"scenario_id": "future.unknown", "status": "passed", "output": {}}
        )
        with self.assertRaisesRegex(ConformanceReportError, "scenario set mismatch"):
            evaluate_implementation_report(MANIFEST_PATH, unknown)

        duplicate = self.passing_report("client.core")
        duplicate["results"].append(deepcopy(duplicate["results"][0]))
        with self.assertRaisesRegex(ConformanceReportError, "duplicate scenario"):
            evaluate_implementation_report(MANIFEST_PATH, duplicate)

    def test_stale_suite_and_incomplete_provenance_are_rejected(self) -> None:
        stale = self.passing_report("server.core")
        stale["suite_digest"] = "0" * 64
        with self.assertRaisesRegex(ConformanceReportError, "different conformance"):
            evaluate_implementation_report(MANIFEST_PATH, stale)

        incomplete = self.passing_report("server.core")
        del incomplete["implementation"]["layer"]
        with self.assertRaisesRegex(ConformanceReportError, "provenance"):
            evaluate_implementation_report(MANIFEST_PATH, incomplete)

        invalid_layer = self.passing_report("server.core")
        invalid_layer["implementation"]["layer"] = "unit"
        with self.assertRaisesRegex(ConformanceReportError, "layer is invalid"):
            evaluate_implementation_report(MANIFEST_PATH, invalid_layer)

        wrong_platform = self.passing_report("server.core")
        wrong_platform["implementation"]["platform"] = "apple"
        with self.assertRaisesRegex(ConformanceReportError, "cannot claim"):
            evaluate_implementation_report(MANIFEST_PATH, wrong_platform)

        missing_status = self.passing_report("server.core")
        del missing_status["results"][0]["status"]
        with self.assertRaisesRegex(ConformanceReportError, "may not be skipped"):
            evaluate_implementation_report(MANIFEST_PATH, missing_status)

    def test_semantic_mismatch_fails_with_stable_digests(self) -> None:
        report = self.passing_report("client.core")
        report["results"][0]["output"]["values"]["account.tier"] = "free"

        evaluation = evaluate_implementation_report(MANIFEST_PATH, report)

        self.assertFalse(evaluation.passed)
        failed = [result for result in evaluation.results if not result.passed]
        self.assertEqual(
            [result.scenario_id for result in failed],
            ["annotations.outer_first"],
        )
        self.assertEqual(failed[0].reasons, ("output_mismatch",))
        self.assertNotEqual(failed[0].expected_digest, failed[0].actual_digest)

    def test_privacy_canaries_never_appear_in_reference_outputs(self) -> None:
        evaluation = run_suite(MANIFEST_PATH)
        for result in evaluation.results:
            scenario = self.scenarios_by_id[result.scenario_id]
            serialized = json.dumps(result.actual, sort_keys=True)
            for token in scenario.get("forbidden_tokens", []):
                with self.subTest(scenario=result.scenario_id, token=token):
                    self.assertNotIn(token, serialized)

    def test_sdk_output_containing_a_privacy_canary_cannot_pass(self) -> None:
        report = self.passing_report("client.core")
        trace = next(
            result
            for result in report["results"]
            if result["scenario_id"] == "trace.first_party_propagation"
        )
        trace["output"]["leaked_baggage"] = "never-forward"

        evaluation = evaluate_implementation_report(MANIFEST_PATH, report)

        failed = [result for result in evaluation.results if not result.passed]
        self.assertEqual(len(failed), 1)
        self.assertEqual(
            failed[0].reasons,
            ("output_mismatch", "forbidden_token:never-forward"),
        )

    def test_offline_replay_and_sampling_edge_cases_are_pinned(self) -> None:
        outputs = {
            result.scenario_id: result.actual
            for result in run_suite(MANIFEST_PATH).results
        }
        offline = outputs["offline.at_least_once_recovery"]
        self.assertEqual(
            offline["export_attempts"],
            [
                ["record-action", "record-impression"],
                ["record-action", "record-impression"],
            ],
        )
        self.assertEqual(offline["dropped"][0]["record_id"], "record-replay")

        replay = outputs["replay.monotonic_alignment"]
        self.assertEqual(replay["aligned_facts"][1]["chunk_id"], "chunk-b")
        self.assertEqual(replay["clock_basis"], "boot_monotonic")

        sampling = outputs["sampling.independent_streams"]["results"]
        self.assertFalse(sampling[0]["trace_sampled"])
        self.assertTrue(sampling[0]["decisions"]["behavior"]["kept"])
        self.assertTrue(sampling[1]["trace_sampled"])
        self.assertFalse(sampling[1]["decisions"]["behavior"]["kept"])

    def test_suite_digest_binds_expected_outputs_and_canaries(self) -> None:
        changed_scenarios = deepcopy(self.scenarios)
        changed_scenarios[0]["expect"]["values"]["account.tier"] = "changed"
        self.assertNotEqual(
            self.digest, suite_digest(self.manifest, changed_scenarios)
        )

        changed_canaries = deepcopy(self.scenarios)
        changed_canaries[3]["forbidden_tokens"].append("new-secret")
        self.assertNotEqual(
            self.digest, suite_digest(self.manifest, changed_canaries)
        )

    def test_manifest_paths_cannot_escape_the_versioned_suite(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = deepcopy(self.manifest)
            manifest["scenarios"][0]["path"] = "../outside.json"
            path = root / "manifest.json"
            path.write_text(json.dumps(manifest), encoding="utf-8")

            with self.assertRaisesRegex(ConformanceSuiteError, "escapes"):
                load_suite(path)


if __name__ == "__main__":
    unittest.main()
