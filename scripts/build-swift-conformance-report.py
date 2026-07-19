#!/usr/bin/env python3
"""Bind Swift-computed scenario outputs to the exact shared suite digest."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from contracts.conformance.v1.reference import load_suite, suite_digest  # noqa: E402


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("swift_output", type=Path)
    parser.add_argument("report", type=Path)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=ROOT / "conformance" / "v1" / "manifest.json",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    with args.swift_output.open(encoding="utf-8") as handle:
        computed = json.load(handle)
    if not isinstance(computed, dict):
        raise ValueError("Swift output must be an object")
    manifest, scenarios = load_suite(args.manifest)
    report = {
        "suite_version": manifest["suite_version"],
        "suite_digest": suite_digest(manifest, scenarios),
        "profile": computed["profile"],
        "implementation": {
            "platform": "apple",
            "adapter": "swift-6.4",
            "sdk_version": "0.1.0-dev",
            "layer": "model",
        },
        "results": computed["results"],
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
