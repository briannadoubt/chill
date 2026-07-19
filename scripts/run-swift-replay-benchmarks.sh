#!/bin/sh
set -eu

workspace_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$workspace_root/sdk/swift"

swift run -c release \
  --scratch-path /tmp/chill-swift-replay-benchmarks \
  ChillReplayBenchmarks
