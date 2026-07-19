#!/bin/sh
set -eu

workspace_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
release_report=""
ios_test_destination=${CHILL_IOS_SIMULATOR_DESTINATION:-platform=iOS Simulator,name=iPhone 17,OS=latest}

if [ "$#" -gt 0 ]; then
  if [ "$#" -ne 2 ] || [ "$1" != "--release-report" ]; then
    echo "usage: $0 [--release-report apple-device-report.json]" >&2
    exit 2
  fi
  release_report=$2
fi

report=$(mktemp /tmp/chill-apple-conformance.XXXXXX)
core_benchmark=$(mktemp /tmp/chill-core-benchmark.XXXXXX)
replay_benchmark=$(mktemp /tmp/chill-replay-benchmark.XXXXXX)
trap 'rm -f "$report" "$core_benchmark" "$replay_benchmark"' EXIT

cd "$workspace_root"
scripts/test-swift-sdk.sh

cd "$workspace_root/sdk/swift/Examples"
swift build --scratch-path /tmp/chill-swift-examples-macos

cd "$workspace_root/sdk/swift"
xcodebuild \
  -scheme Chill-Package \
  -destination "$ios_test_destination" \
  -derivedDataPath /tmp/chill-swift-validation-ios-tests \
  CODE_SIGNING_ALLOWED=NO \
  test >/dev/null
xcodebuild \
  -scheme Chill \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chill-swift-validation-ios \
  CODE_SIGNING_ALLOWED=NO \
  build >/dev/null
xcodebuild \
  -scheme ChillReplay \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chill-swift-validation-ios-replay \
  CODE_SIGNING_ALLOWED=NO \
  build >/dev/null

cd "$workspace_root/sdk/swift/Examples"
xcodebuild \
  -scheme ChillSwiftUISample \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chill-swift-sample-swiftui-ios \
  CODE_SIGNING_ALLOWED=NO \
  build >/dev/null
xcodebuild \
  -scheme ChillUIKitSample \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chill-swift-sample-uikit-ios \
  CODE_SIGNING_ALLOWED=NO \
  build >/dev/null

cd "$workspace_root"
scripts/run-swift-conformance.sh "$report" >/dev/null
if grep -q 'SECRET-' "$report"; then
  echo "privacy canary escaped into the Swift conformance report" >&2
  exit 1
fi

scripts/run-swift-core-benchmarks.sh > "$core_benchmark"
scripts/run-swift-replay-benchmarks.sh > "$replay_benchmark"

python3 - "$core_benchmark" "$replay_benchmark" <<'PY'
import json
from pathlib import Path
import sys

core = json.loads(Path(sys.argv[1]).read_text())
replay = json.loads(Path(sys.argv[2]).read_text())
print(
    "Swift host gates passed: "
    f"disabled p99={core['disabled_p99_nanoseconds_per_call']:.2f}ns, "
    f"replay p95={replay['p95_milliseconds']:.4f}ms, "
    f"replay p99={replay['p99_milliseconds']:.4f}ms"
)
PY

if [ -n "$release_report" ]; then
  python3 scripts/evaluate-sdk-budgets.py "$release_report"
else
  echo "Physical-device release attestation was not requested; host gates are presubmit evidence only."
fi
