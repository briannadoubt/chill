#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
aar="$root/android/build/outputs/aar/android-release.aar"
jar="$root/core/build/libs/core-0.1.0.jar"
maximum=$((512 * 1024))
bytes=$(($(wc -c < "$aar") + $(wc -c < "$jar")))
if (( bytes > maximum )); then
  echo "Android SDK artifacts $bytes bytes exceed $maximum" >&2
  exit 1
fi
echo "Android SDK artifacts $bytes bytes / $maximum byte budget"
