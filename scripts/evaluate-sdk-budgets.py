#!/usr/bin/env python3
"""Evaluate one SDK benchmark report against the checked release budgets."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from contracts.budgets.v1 import (  # noqa: E402
    BudgetCatalogError,
    BudgetReportError,
    evaluate_report,
    load_catalog,
)


DEFAULT_CATALOG = ROOT / "budgets" / "sdk" / "v1" / "budgets.json"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="benchmark report JSON")
    parser.add_argument(
        "--catalog",
        type=Path,
        default=DEFAULT_CATALOG,
        help="budget catalog JSON (defaults to the checked V1 catalog)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        catalog = load_catalog(args.catalog)
        with args.report.open(encoding="utf-8") as handle:
            report = json.load(handle)
        if not isinstance(report, dict):
            raise BudgetReportError("benchmark report must be an object")
        evaluation = evaluate_report(catalog, report)
    except (OSError, json.JSONDecodeError, BudgetCatalogError, BudgetReportError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    print(json.dumps(evaluation.as_dict(), indent=2, sort_keys=True))
    return 0 if evaluation.passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
