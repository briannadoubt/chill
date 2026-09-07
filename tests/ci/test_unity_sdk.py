from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import unittest


class UnitySdkValidationTests(unittest.TestCase):
    def test_unity_package_contract(self) -> None:
        root = Path(__file__).resolve().parents[2]
        result = subprocess.run(
            [sys.executable, "scripts/validate-unity-sdk.py"],
            cwd=root,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Unity SDK validation passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
