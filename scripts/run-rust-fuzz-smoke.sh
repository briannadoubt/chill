#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
seconds="${CHILL_FUZZ_SECONDS:-30}"
case "$seconds" in
  ''|*[!0-9]*) echo "CHILL_FUZZ_SECONDS must be a positive integer" >&2; exit 2 ;;
esac
test "$seconds" -ge 1

cd "$root/backend/fuzz"
nightly_bin="$(dirname "$(rustup which --toolchain nightly rustc)")"
export PATH="$nightly_bin:$HOME/.cargo/bin:$PATH"
for target in credential_parser ingest_candidate query_plan; do
  cargo fuzz run "$target" -- -max_total_time="$seconds" -timeout=5
done
