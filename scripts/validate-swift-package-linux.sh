#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE="$ROOT/sdk/swift"
SCRATCH="$PACKAGE/.build/ci-linux"

swift --version | grep -E 'Swift version 6\.4|Swift version 6\.4-dev'
swift format lint --recursive "$PACKAGE/Sources" "$PACKAGE/Tests"

resolved_before="$(sha256sum "$PACKAGE/Package.resolved")"
(
  cd "$PACKAGE"
  swift package dump-package > /tmp/chill-swift-package.json
  swift package resolve --scratch-path "$SCRATCH"
)
resolved_after="$(sha256sum "$PACKAGE/Package.resolved")"

if [[ "$resolved_before" != "$resolved_after" ]]; then
  echo "Swift dependency resolution changed Package.resolved" >&2
  exit 1
fi

if grep -R -Eiq \
  'public[^[:cntrl:]]*(func|static func)[[:space:]]+(track|record|capture)[[:space:]]*\(' \
  "$PACKAGE/Sources"; then
  echo "public imperative telemetry API detected" >&2
  exit 1
fi

echo swift-package-linux-contract-ok
