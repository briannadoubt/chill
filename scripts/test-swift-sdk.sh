#!/bin/sh
set -eu

workspace_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$workspace_root/sdk/swift"

swift format lint --recursive Sources Tests
swift test --scratch-path /tmp/chill-swift-tests

symbols=$(mktemp -d /tmp/chill-swift-symbols.XXXXXX)
trap 'rm -rf "$symbols"' EXIT
swift package \
  --scratch-path /tmp/chill-swift-symbol-build \
  dump-symbol-graph \
  --minimum-access-level public \
  --output-dir "$symbols"

if grep -Eiq '"title":"(track|record|capture)\(' "$symbols"/*.json; then
  echo "public imperative telemetry API detected" >&2
  exit 1
fi
