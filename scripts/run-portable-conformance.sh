#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
if [ "$#" -gt 1 ]; then
  echo "usage: $0 [implementation-report.json]" >&2
  exit 2
fi
python3 -m unittest tests.conformance.test_cross_platform_suite_v2
python3 - <<'PY'
from pathlib import Path
from contracts.conformance.v2 import run_suite
evaluation = run_suite(Path("conformance/v2/manifest.json"))
assert evaluation.passed, evaluation.as_dict()
print("portable V2 reference passed")
PY
if [ "$#" -eq 1 ]; then
  python3 - "$1" <<'PY'
import json
from pathlib import Path
import sys
from contracts.conformance.v2 import evaluate_implementation_report

with Path(sys.argv[1]).open(encoding="utf-8") as handle:
    report = json.load(handle)
evaluation = evaluate_implementation_report(
    Path("conformance/v2/manifest.json"), report
)
print(json.dumps(evaluation.as_dict(), indent=2, sort_keys=True))
raise SystemExit(0 if evaluation.passed else 1)
PY
fi
