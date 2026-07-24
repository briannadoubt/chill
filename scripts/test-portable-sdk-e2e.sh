#!/usr/bin/env bash

# Sends the portable SDK fixture to an already-running OTLP/HTTP endpoint.
# It is intentionally safe in local checkouts: no endpoint or key means skip.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE="$ROOT/backend/testdata/portable-sdk-otlp.json"

if [[ -z "${CHILL_OTLP_ENDPOINT:-}" || -z "${CHILL_API_KEY:-}" ]]; then
  echo "portable-sdk-e2e-skipped: CHILL_OTLP_ENDPOINT and CHILL_API_KEY are required"
  exit 0
fi

endpoint="${CHILL_OTLP_ENDPOINT%/}"
if [[ "$endpoint" != */v1/logs ]]; then
  endpoint="$endpoint/v1/logs"
fi

response_file="$(mktemp -t chill-portable-sdk-response.XXXXXX)"
collection_file="$(mktemp -t chill-portable-sdk-collection.XXXXXX)"
request_fixture="$(mktemp -t chill-portable-sdk-fixture.XXXXXX)"
cleanup() { rm -f "$response_file" "$collection_file" "$request_fixture"; }
trap cleanup EXIT

collection_endpoint="${endpoint%/v1/logs}/v1/chill/collection-state"
collection_status="$(curl --silent --show-error --output "$collection_file" --write-out '%{http_code}' \
  --header "authorization: Bearer $CHILL_API_KEY" \
  "$collection_endpoint")"
if [[ "$collection_status" != "200" ]]; then
  echo "portable-sdk-e2e-failed: collection state returned $collection_status" >&2
  exit 1
fi
policy_version="$(python3 - "$collection_file" <<'PY'
import json
from pathlib import Path
import re
import sys

state = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
version = state.get("policy_version")
disabled = state.get("disabled_capture_classes", [])
if state.get("enabled") is not True or "analytics" in disabled:
    raise SystemExit("portable-sdk-e2e-failed: analytics collection is disabled")
if not isinstance(version, str) or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", version) is None:
    raise SystemExit("portable-sdk-e2e-failed: invalid active policy version")
print(version)
PY
)"
python3 - "$FIXTURE" "$request_fixture" "$policy_version" <<'PY'
import json
from pathlib import Path
import secrets
import sys
import time
import uuid


def uuid7(timestamp_ms: int) -> str:
    value = bytearray(timestamp_ms.to_bytes(6, "big") + secrets.token_bytes(10))
    value[6] = (value[6] & 0x0F) | 0x70
    value[8] = (value[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(value)))

request = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
now = time.time_ns()
trace_id = secrets.token_hex(16)
ordinal = 0
for resource in request["resourceLogs"]:
    for scope in resource["scopeLogs"]:
        for record in scope["logRecords"]:
            record_id = uuid7(now // 1_000_000 + ordinal)
            record["timeUnixNano"] = str(now + ordinal)
            record["observedTimeUnixNano"] = str(now + ordinal)
            record["traceId"] = trace_id
            record["spanId"] = secrets.token_hex(8)
            for attribute in record["attributes"]:
                if attribute["key"] == "chill.privacy.policy_version":
                    attribute["value"] = {"stringValue": sys.argv[3]}
                elif attribute["key"] in {"chill.record.id", "chill.subject.id"}:
                    attribute["value"] = {"stringValue": record_id}
            ordinal += 1
Path(sys.argv[2]).write_text(json.dumps(request, separators=(",", ":")) + "\n", encoding="utf-8")
PY

status="$(curl --silent --show-error --output "$response_file" --write-out '%{http_code}' \
  --request POST "$endpoint" \
  --header "authorization: Bearer $CHILL_API_KEY" \
  --header 'content-type: application/json' \
  --header 'x-chill-schema-version: 1.0.0' \
  --data-binary "@$request_fixture")"
if [[ ! "$status" =~ ^2[0-9][0-9]$ ]]; then
  echo "portable-sdk-e2e-failed: OTLP/HTTP returned $status" >&2
  exit 1
fi

for command in npm node deno bun cargo; do
  if ! command -v "$command" >/dev/null; then
    echo "portable-sdk-e2e-failed: required command is unavailable: $command" >&2
    exit 1
  fi
done

export CHILL_OTLP_ENDPOINT="$endpoint"
export CHILL_POLICY_VERSION="$policy_version"
(cd "$ROOT/sdk/js" && npm run --silent ingest:smoke --workspace @chill-observability/runtime)
(cd "$ROOT/sdk/js/packages/runtime" && deno run --allow-read=dist --allow-env=CHILL_OTLP_ENDPOINT,CHILL_API_KEY,CHILL_POLICY_VERSION --allow-net scripts/ingest-smoke.mjs deno ../dist/deno.js)
(cd "$ROOT/sdk/js/packages/runtime" && bun scripts/ingest-smoke.mjs bun ../dist/bun.js)
(cd "$ROOT/sdk/rust" && cargo run --quiet --locked --example ingest-smoke)

echo "portable-sdk-e2e-ok: fixture plus Node, Deno, Bun, and Rust exporters were acknowledged ($status)"
