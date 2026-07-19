#!/usr/bin/env python3
"""Run Chill V1 reference fixtures or evaluate one SDK conformance report."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from contracts.conformance.v1 import (  # noqa: E402
    ConformanceReportError,
    ConformanceSuiteError,
    evaluate_implementation_report,
    run_suite,
)


DEFAULT_MANIFEST = ROOT / "conformance" / "v1" / "manifest.json"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "report",
        nargs="?",
        type=Path,
        help="SDK implementation report JSON; omit to run the reference model",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=DEFAULT_MANIFEST,
        help="suite manifest JSON (defaults to the checked V1 manifest)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.report is None:
            evaluation = run_suite(args.manifest)
        else:
            with args.report.open(encoding="utf-8") as handle:
                report = json.load(handle)
            if not isinstance(report, dict):
                raise ConformanceReportError("report must be a JSON object")
            evaluation = evaluate_implementation_report(args.manifest, report)
    except (
        OSError,
        json.JSONDecodeError,
        ConformanceSuiteError,
        ConformanceReportError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    print(json.dumps(evaluation.as_dict(), indent=2, sort_keys=True))
    return 0 if evaluation.passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
