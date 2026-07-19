from __future__ import annotations

import json
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
POLICY_ROOT = ROOT / "policies" / "privacy" / "v1"


class DataClassificationContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.catalog = json.loads(
            (POLICY_ROOT / "data-classification.json").read_text()
        )
        cls.schema = json.loads(
            (POLICY_ROOT / "environment-policy.schema.json").read_text()
        )
        cls.policy = json.loads(
            (POLICY_ROOT / "default-environment-policy.json").read_text()
        )

    def test_catalog_covers_every_required_source_category(self) -> None:
        self.assertEqual(self.catalog["schema_version"], "1.0.0")
        self.assertEqual(
            set(self.catalog["source_categories"]),
            {
                "identifiers",
                "content",
                "text",
                "network_fields",
                "device_data",
                "custom_annotations",
                "secrets",
                "credentials",
            },
        )
        for name, category in self.catalog["source_categories"].items():
            with self.subTest(category=name):
                self.assertIn(
                    category["classification"], self.catalog["classifications"]
                )
                self.assertIn(category["default_disposition"], {"omit", "mask"})
                self.assertTrue(category["examples"])

    def test_environment_policy_is_default_deny_and_schema_complete(self) -> None:
        self.assertEqual(self.policy["schema_version"], "1.0.0")
        self.assertEqual(self.policy["default_disposition"], "omit")
        self.assertEqual(self.schema["additionalProperties"], False)
        self.assertEqual(set(self.schema["required"]), set(self.policy))

        name_pattern = re.compile(
            self.schema["properties"]["annotation_allowlist"]["propertyNames"][
                "pattern"
            ]
        )
        allowed_classifications = set(
            self.schema["properties"]["annotation_allowlist"][
                "additionalProperties"
            ]["enum"]
        )
        for name, classification in self.policy["annotation_allowlist"].items():
            with self.subTest(annotation=name):
                self.assertRegex(name, name_pattern)
                self.assertIn(classification, allowed_classifications)
                self.assertTrue(
                    self.catalog["classifications"][classification][
                        "annotation_eligible"
                    ]
                )

    def test_secrets_credentials_and_raw_replay_content_cannot_be_allowed(self) -> None:
        rules = self.catalog["non_overridable_rules"]
        self.assertEqual(rules["credentials"], "omit")
        self.assertEqual(rules["secrets"], "omit")
        self.assertEqual(rules["secure_input"], "mask")
        self.assertEqual(rules["clipboard"], "mask")
        self.assertEqual(rules["replay_masking_stage"], "before_buffer")

        replay = self.policy["replay"]
        self.assertTrue(replay["mask_at_source"])
        self.assertEqual(replay["custom_drawing"], "block")
        for field in (
            "text",
            "form_values",
            "accessibility_text",
            "secure_input",
            "pixels",
        ):
            self.assertEqual(replay[field], "mask")

    def test_every_canonical_fixture_classifies_every_annotation(self) -> None:
        eligible = {
            name
            for name, definition in self.catalog["classifications"].items()
            if definition["annotation_eligible"]
        }
        for path in sorted((ROOT / "examples" / "behavior" / "v1").glob("*.json")):
            with self.subTest(path=path.name):
                record = json.loads(path.read_text())
                annotations = record["annotations"]
                classifications = record["privacy"][
                    "annotation_classifications"
                ]
                self.assertEqual(set(classifications), set(annotations))
                self.assertTrue(set(classifications.values()) <= eligible)


if __name__ == "__main__":
    unittest.main()
