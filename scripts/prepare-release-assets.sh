#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="${1:-$(tr -d '\n' < "$ROOT/VERSION")}"
output="${2:-$ROOT/dist}"
commit="$(git -C "$ROOT" rev-parse HEAD)"
source_date="$(git -C "$ROOT" show -s --format=%cI "$commit")"

if [[ ! "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]]; then
  echo "release asset version is not canonical: $version" >&2
  exit 2
fi

rm -rf "$output"
mkdir -p "$output"
temporary="$(mktemp -d -t chill-release.XXXXXX)"
trap 'rm -rf "$temporary"' EXIT

git -C "$ROOT" archive \
  --format=tar \
  --mtime="$source_date" \
  --prefix="ChillSwift-$version/" \
  --add-file=LICENSE \
  --add-file=NOTICE \
  "$commit:sdk/swift" > "$temporary/swift.tar"
gzip -n -9 < "$temporary/swift.tar" > "$output/chill-swift-$version.tar.gz"

git -C "$ROOT" archive \
  --format=tar \
  --mtime="$source_date" \
  --prefix="ChillWeb-$version/" \
  --add-file=LICENSE \
  --add-file=NOTICE \
  "$commit:sdk/web" > "$temporary/web.tar"
gzip -n -9 < "$temporary/web.tar" > "$output/chill-web-$version.tar.gz"

git -C "$ROOT" archive \
  --format=tar \
  --mtime="$source_date" \
  --prefix="ChillAndroid-$version/" \
  --add-file=LICENSE \
  --add-file=NOTICE \
  "$commit:sdk/android" > "$temporary/android.tar"
gzip -n -9 < "$temporary/android.tar" > "$output/chill-android-$version.tar.gz"

git -C "$ROOT" archive \
  --format=tar \
  --mtime="$source_date" \
  --prefix="ChillRust-$version/" \
  --add-file=LICENSE \
  --add-file=NOTICE \
  "$commit:sdk/rust" > "$temporary/rust.tar"
gzip -n -9 < "$temporary/rust.tar" > "$output/chill-rust-$version.tar.gz"

git -C "$ROOT" archive \
  --format=tar \
  --mtime="$source_date" \
  --prefix="ChillJavaScript-$version/" \
  --add-file=LICENSE \
  --add-file=NOTICE \
  "$commit:sdk/js" > "$temporary/javascript.tar"
gzip -n -9 < "$temporary/javascript.tar" > "$output/chill-javascript-$version.tar.gz"

git -C "$ROOT" archive \
  --format=tar \
  --mtime="$source_date" \
  --prefix="ChillTauri-$version/" \
  --add-file=LICENSE \
  --add-file=NOTICE \
  "$commit:sdk/tauri" > "$temporary/tauri.tar"
gzip -n -9 < "$temporary/tauri.tar" > "$output/chill-tauri-$version.tar.gz"

git -C "$ROOT" archive \
  --format=tar \
  --mtime="$source_date" \
  --prefix="ChillUnity-$version/" \
  --add-file=LICENSE \
  --add-file=NOTICE \
  "$commit:sdk/unity" > "$temporary/unity.tar"
gzip -n -9 < "$temporary/unity.tar" > "$output/chill-unity-$version.tar.gz"

git -C "$ROOT" archive \
  --format=tar \
  --mtime="$source_date" \
  --prefix="ChillContracts-$version/" \
  "$commit" -- \
  LICENSE NOTICE budgets conformance contracts examples/behavior examples/otlp policies schemas \
  requirements-contract.txt > "$temporary/contracts.tar"
gzip -n -9 < "$temporary/contracts.tar" > "$output/chill-contracts-$version.tar.gz"

python3 - "$output" "$version" "$commit" <<'PY'
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import sys

output = Path(sys.argv[1])
artifacts = []
for path in sorted(output.glob("*.tar.gz")):
    body = path.read_bytes()
    artifacts.append({
        "bytes": len(body),
        "name": path.name,
        "sha256": hashlib.sha256(body).hexdigest(),
    })
manifest = {
    "artifacts": artifacts,
    "commit": sys.argv[3],
    "format": "chill-release-assets-v1",
    "version": sys.argv[2],
}
(output / "manifest.json").write_text(
    json.dumps(manifest, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

echo "$output"
