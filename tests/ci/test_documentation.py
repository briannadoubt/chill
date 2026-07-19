from __future__ import annotations

import unittest

from scripts import validate_documentation


class DocumentationTests(unittest.TestCase):
    def test_required_guides_have_valid_local_links_and_semantics(self) -> None:
        self.assertEqual(validate_documentation.verify(), [])

    def test_missing_local_link_is_rejected(self) -> None:
        document = validate_documentation.ROOT / "docs" / "example.md"
        errors = validate_documentation.local_link_errors(
            document,
            "[missing](does-not-exist.md)",
        )
        self.assertEqual(len(errors), 1)


if __name__ == "__main__":
    unittest.main()
