#!/usr/bin/env python3
"""Aggregate verified Apple observation fragments into a budget report."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from validation.apple.v1.release_validation import (  # noqa: E402
    ReleaseValidationError,
    aggregate_fragments,
    validate_inputs,
)


def _fragments(values: list[str]) -> list[Path]:
    result: list[Path] = []
    for value in values:
        path = Path(value)
        if path.is_dir():
            result.extend(sorted(path.rglob("*.json")))
        else:
            result.append(path)
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("fragments", nargs="+", help="fragment files or directories")
    parser.add_argument(
        "--manifest", default="validation/apple/v1/profiler-scenarios.json"
    )
    parser.add_argument("--catalog")
    parser.add_argument("--output", help="write report atomically; default stdout")
    arguments = parser.parse_args()
    try:
        manifest, catalog, specs, manifest_hash, catalog_hash = validate_inputs(
            arguments.manifest, arguments.catalog
        )
        report = aggregate_fragments(
            manifest,
            catalog,
            specs,
            manifest_hash,
            catalog_hash,
            _fragments(arguments.fragments),
        )
        encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if arguments.output:
            output = Path(arguments.output)
            temporary = output.with_name(f".{output.name}.tmp")
            temporary.write_text(encoded, encoding="utf-8")
            temporary.replace(output)
        else:
            sys.stdout.write(encoded)
    except (OSError, ReleaseValidationError, ValueError) as error:
        print(f"apple release aggregation failed: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
