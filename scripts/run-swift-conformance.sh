#!/bin/sh
set -eu

workspace_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
manifest="$workspace_root/conformance/v1/manifest.json"
report="${1:-$workspace_root/conformance/v1/reports/apple.swift.replay.json}"
raw=$(mktemp /tmp/chill-swift-conformance.XXXXXX)
trap 'rm -f "$raw"' EXIT

cd "$workspace_root/sdk/swift"
swift run \
  --scratch-path /tmp/chill-swift-conformance-build \
  ChillConformanceRunner "$manifest" client.replay > "$raw"

cd "$workspace_root"
python3 scripts/build-swift-conformance-report.py \
  "$raw" \
  "$report" \
  --manifest "$manifest"
python3 scripts/run-conformance-suite.py "$report"
