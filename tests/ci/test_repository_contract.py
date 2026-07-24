from __future__ import annotations

from pathlib import Path
import subprocess
import unittest

from scripts import repository_contract


class RepositoryContractTests(unittest.TestCase):
    def test_manifest_is_sorted_and_digest_bound(self) -> None:
        manifest = repository_contract.manifest_payload()
        paths = [entry["path"] for entry in manifest["files"]]
        self.assertEqual(paths, sorted(paths))
        self.assertEqual(len(paths), len(set(paths)))
        for entry in manifest["files"]:
            self.assertRegex(entry["sha256"], r"^[0-9a-f]{64}$")
            self.assertGreater(entry["bytes"], 0)

    def test_contract_majors_do_not_mutate_v1_inputs(self) -> None:
        v1_paths = {
            entry["path"] for entry in repository_contract.manifest_payload(1)["files"]
        }
        self.assertFalse(any("/v2/" in path for path in v1_paths))
        if any(
            "/v2/" in path.as_posix()
            for path in repository_contract.tracked_contract_paths(1)
        ):
            self.fail("V2 input leaked into the V1 manifest")
        tracked = subprocess.run(
            ["git", "ls-files", "--", "conformance/v2", "contracts/*/v2", "budgets/sdk/v2"],
            cwd=repository_contract.ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines()
        if tracked:
            v2 = repository_contract.manifest_payload(2)
            self.assertTrue(v2["files"])
            self.assertTrue(all("/v2/" in entry["path"] for entry in v2["files"]))
            self.assertEqual(
                v2["extends"]["manifest"],
                "contracts/generated/manifest.v1.json",
            )

    def test_mutable_action_reference_is_rejected(self) -> None:
        workflow = repository_contract.ROOT / ".github" / "workflows" / "example.yml"
        errors = repository_contract.validate_action_pins(
            workflow,
            "steps:\n  - uses: actions/checkout@v7\n",
        )
        self.assertEqual(len(errors), 1)

    def test_pinned_action_reference_with_release_comment_is_accepted(self) -> None:
        workflow = repository_contract.ROOT / ".github" / "workflows" / "example.yml"
        errors = repository_contract.validate_action_pins(
            workflow,
            "steps:\n"
            "  - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0\n",
        )
        self.assertEqual(errors, [])

    def test_version_contract_rejects_mismatched_tag(self) -> None:
        version = (repository_contract.ROOT / "VERSION").read_text().strip()
        self.assertTrue(repository_contract.SEMVER.fullmatch(version))
        self.assertNotEqual(repository_contract.validate_version("v9.9.9"), [])

    def test_public_repository_contract_is_complete(self) -> None:
        self.assertEqual(repository_contract.validate_public_repository(), [])


if __name__ == "__main__":
    unittest.main()
