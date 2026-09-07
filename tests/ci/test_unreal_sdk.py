"""Contract coverage for the source-only Unreal plugin."""

from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class UnrealSdkValidationTests(unittest.TestCase):
    def test_unreal_plugin_contract(self) -> None:
        completed = subprocess.run(
            [sys.executable, str(ROOT / "scripts" / "validate-unreal-sdk.py")],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)


if __name__ == "__main__":
    unittest.main()
