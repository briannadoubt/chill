from __future__ import annotations

import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
ATTESTATION = (
    ROOT
    / "validation"
    / "apple"
    / "physical"
    / "attestations"
    / "2026-07-15-iphone13-smoke.json"
)
PROFILER_MANIFEST = ROOT / "validation" / "apple" / "v1" / "profiler-scenarios.json"


class ApplePhysicalSmokeAttestationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.raw = ATTESTATION.read_text(encoding="utf-8")
        cls.attestation = json.loads(cls.raw)

    def test_attestation_is_sanitized_and_bound_to_the_profiler_manifest(self) -> None:
        forbidden_fragments = (
            "DEVELOPMENT_TEAM",
            "DevelopmentTeam",
            "provisioning",
            "00008110-",
            "A578FCE2-",
        )
        for fragment in forbidden_fragments:
            with self.subTest(fragment=fragment):
                self.assertNotIn(fragment, self.raw)

        digest = hashlib.sha256(PROFILER_MANIFEST.read_bytes()).hexdigest()
        self.assertEqual(self.attestation["profiler_manifest_sha256"], digest)
        self.assertTrue(self.attestation["device"]["physical"])
        self.assertEqual(self.attestation["build"]["configuration"], "release")

    def test_smoke_result_cannot_claim_release_eligibility(self) -> None:
        interaction = self.attestation["interaction"]
        launch = self.attestation["launch_diagnostic"]
        release = self.attestation["release_gate"]

        self.assertEqual(interaction["release_ui_tests"], 1)
        self.assertEqual(interaction["passed"], 1)
        self.assertTrue(interaction["declarative_action_activated"])
        self.assertEqual(launch["candidate_samples"], 30)
        self.assertEqual(launch["baseline_samples"], 30)
        self.assertFalse(launch["interleaved"])
        self.assertFalse(launch["release_budget_eligible"])
        self.assertAlmostEqual(
            launch["candidate_mean_milliseconds"]
            - launch["baseline_mean_milliseconds"],
            launch["mean_delta_milliseconds"],
            places=3,
        )
        self.assertFalse(release["passed"])
        self.assertGreaterEqual(len(release["reasons"]), 5)

    def test_smoke_attestation_is_not_a_release_budget_report(self) -> None:
        self.assertNotIn("catalog_version", self.attestation)
        self.assertNotIn("profile", self.attestation)
        self.assertNotIn("measurements", self.attestation)


if __name__ == "__main__":
    unittest.main()
