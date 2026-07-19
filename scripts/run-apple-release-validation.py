#!/usr/bin/env python3
"""Create a reproducible Apple release run plan and optionally preflight it."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from validation.apple.v1.release_validation import (  # noqa: E402
    ReleaseValidationError,
    execute_xctest_plan,
    generate_plan,
    manifest_xcode_configuration,
    physical_preflight,
    validate_inputs,
)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--manifest", default="validation/apple/v1/profiler-scenarios.json"
    )
    parser.add_argument("--catalog")
    parser.add_argument("--seed", type=int, default=20260717)
    parser.add_argument("--output", help="write plan to this path; default stdout")
    parser.add_argument("--execute", action="store_true", help="perform physical preflight")
    parser.add_argument("--device", help="physical device ID (never written to output)")
    parser.add_argument("--team", help="development team (never written to output)")
    parser.add_argument(
        "--evidence-directory",
        default="validation/apple/physical/evidence",
        help="ignored raw evidence root (never written to output)",
    )
    arguments = parser.parse_args()
    try:
        manifest, _catalog, _specs, manifest_hash, catalog_hash = validate_inputs(
            arguments.manifest, arguments.catalog
        )
        plan = generate_plan(manifest, manifest_hash, catalog_hash, arguments.seed)
        if arguments.execute:
            if not arguments.device or not arguments.team:
                raise ReleaseValidationError("--execute requires --device and --team")
            project, schemes = manifest_xcode_configuration(manifest)
            plan["preflight"] = physical_preflight(
                arguments.device,
                arguments.team,
                ROOT / project,
                schemes,
                manifest["floor_device"],
            )
            plan["execution"] = execute_xctest_plan(
                manifest,
                plan,
                arguments.device,
                arguments.team,
                ROOT / project,
                ROOT / arguments.evidence_directory,
            )
            plan["status"] = plan["execution"]["status"]
        else:
            plan["status"] = "planned; no commands executed"
        encoded = json.dumps(plan, indent=2, sort_keys=True) + "\n"
        if arguments.output:
            output = Path(arguments.output)
            temporary = output.with_name(f".{output.name}.tmp")
            temporary.write_text(encoded, encoding="utf-8")
            temporary.replace(output)
        else:
            sys.stdout.write(encoded)
    except (OSError, ReleaseValidationError, ValueError) as error:
        print(f"apple release planning failed: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
