#!/bin/sh
set -eu

workspace_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$workspace_root/sdk/swift"

exec swift run \
  --configuration release \
  --scratch-path /tmp/chill-swift-benchmarks \
  ChillCoreBenchmarks "${1:-1000000}"
