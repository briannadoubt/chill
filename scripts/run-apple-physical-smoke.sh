#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPEC="$ROOT/validation/apple/physical/project.yml"
PROJECT="$ROOT/validation/apple/physical/ChillPhysicalValidation.xcodeproj"
EVIDENCE_ROOT="$ROOT/validation/apple/physical/evidence"
device=""
team="${CHILL_APPLE_DEVELOPMENT_TEAM:-}"

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --device)
      device="${2:-}"
      shift 2
      ;;
    --team)
      team="${2:-}"
      shift 2
      ;;
    *)
      echo "usage: $0 --device XCODE_DEVICE_ID [--team DEVELOPMENT_TEAM]" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$device" || -z "$team" ]]; then
  echo "a physical Xcode device ID and Apple development team are required" >&2
  exit 2
fi
for command in xcodegen xcodebuild xcrun; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "required command not found: $command" >&2
    exit 1
  }
done

if ! xcrun xctrace list devices 2>/dev/null \
  | grep -F "$device" \
  | grep -Fvq Simulator; then
  echo "device $device is not an available paired physical device" >&2
  exit 1
fi

xcodegen generate --spec "$SPEC" >/dev/null
run_id="$(date -u +%Y%m%dT%H%M%SZ)"
evidence="$EVIDENCE_ROOT/$run_id"
result="$evidence/Interaction.xcresult"
mkdir -p "$evidence"

set -o pipefail
xcodebuild \
  -project "$PROJECT" \
  -scheme ChillPhysicalCandidateValidation \
  -configuration Release \
  -destination "platform=iOS,id=$device" \
  -derivedDataPath /tmp/chill-physical-validation \
  -resultBundlePath "$result" \
  DEVELOPMENT_TEAM="$team" \
  CODE_SIGN_STYLE=Automatic \
  -allowProvisioningUpdates \
  -only-testing:ChillPhysicalCandidateUITests/ChillPhysicalValidationUITests/testInteractionEvidence \
  test 2>&1 | tee "$evidence/interaction.log"

xcrun xcresulttool get test-results summary --path "$result" \
  > "$evidence/interaction-summary.json"
python3 - "$evidence/interaction-summary.json" <<'PY'
import json
from pathlib import Path
import sys

summary = json.loads(Path(sys.argv[1]).read_text())
if summary.get("result") != "Passed" or summary.get("failedTests") != 0:
    raise SystemExit("physical interaction result did not pass")
if summary.get("passedTests") != 1:
    raise SystemExit("physical interaction result must contain exactly one passing test")
PY

echo "apple-physical-smoke-ok: raw evidence is intentionally ignored at $evidence"
