#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKEND="$ROOT/backend"
COMPOSE_FILE="$BACKEND/compose.control-plane.yml"
DUMP_FILE="$(mktemp -t chill-control-plane.XXXXXX.dump)"
SERVER_LOG="$(mktemp -t chill-server.XXXXXX.log)"
SERVER_BINARY="$(mktemp -t chill-server.XXXXXX.bin)"
BOOTSTRAP_CONFIG="$(mktemp -t chill-bootstrap.XXXXXX.json)"
QUERY_PLAN="$(mktemp -t chill-query-plan.XXXXXX.json)"
QUERY_CACHE="$(mktemp -d -t chill-query-cache.XXXXXX)"
DELETION_TARGET="$(mktemp -t chill-deletion-target.XXXXXX.txt)"
PRIVACY_EXPORT_DIR="$(mktemp -d -t chill-privacy-export.XXXXXX)"
SERVER_PID=""

cleanup() {
  local status=$?
  if [[ $status -ne 0 && -s "$SERVER_LOG" ]]; then
    cat "$SERVER_LOG" >&2
  fi
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$DUMP_FILE"
  rm -f "$SERVER_LOG"
  rm -f "$SERVER_BINARY"
  rm -f "$BOOTSTRAP_CONFIG"
  rm -f "$QUERY_PLAN"
  rm -f "$DELETION_TARGET"
  rm -rf "$PRIVACY_EXPORT_DIR"
  rm -rf "$QUERY_CACHE"
  docker compose -f "$COMPOSE_FILE" down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker compose -f "$COMPOSE_FILE" up --detach --wait postgres minio
docker compose -f "$COMPOSE_FILE" run --rm minio-init

export CHILL_DATABASE_URL='postgres://chill_admin:chill_admin_local_only@127.0.0.1:55432/chill?sslmode=disable'
export CHILL_TEST_DATABASE_URL="$CHILL_DATABASE_URL"
export CHILL_TEST_ADMIN_DATABASE_URL="$CHILL_DATABASE_URL"
export CHILL_TEST_RUNTIME_DATABASE_URL='postgres://chill_runtime:chill_runtime_local_only@127.0.0.1:55432/chill?sslmode=disable'
export CHILL_KEY_PEPPER='XFxcXFxcXFxcXFxcXFxcXFxcXFxcXFxcXFxcXFxcXFw='
export AWS_ACCESS_KEY_ID='chill_local'
export AWS_SECRET_ACCESS_KEY='chill_local_secret'
export AWS_EC2_METADATA_DISABLED='true'
export CHILL_OBJECT_STORE_BACKEND='s3'
export CHILL_S3_ENDPOINT='http://127.0.0.1:59000'
export CHILL_S3_BUCKET='chill'
export CHILL_S3_REGION='us-east-1'
export CHILL_S3_PATH_STYLE='true'
export CHILL_TEST_S3_ENDPOINT="$CHILL_S3_ENDPOINT"

cd "$BACKEND"
cargo run --locked --quiet --bin chillctl -- migrate

docker compose -f "$COMPOSE_FILE" exec -T postgres \
  psql --username chill_admin --dbname chill --set ON_ERROR_STOP=1 \
  --command "CREATE ROLE chill_runtime LOGIN PASSWORD 'chill_runtime_local_only' IN ROLE chill_app"

cargo test --locked --workspace -- --include-ignored --test-threads=1
cargo clippy --locked --workspace --all-targets -- -D warnings

# The ignored integration tests intentionally exercise the real control-plane
# database and object store. Reprovision them before the end-to-end release
# fixture so test-owned manifests and scheduled alerts cannot leak across the
# two independently meaningful verification phases.
docker compose -f "$COMPOSE_FILE" down --volumes --remove-orphans
docker compose -f "$COMPOSE_FILE" up --detach --wait postgres minio
docker compose -f "$COMPOSE_FILE" run --rm minio-init
cargo run --locked --quiet --bin chillctl -- migrate
docker compose -f "$COMPOSE_FILE" exec -T postgres \
  psql --username chill_admin --dbname chill --set ON_ERROR_STOP=1 \
  --command "CREATE ROLE chill_runtime LOGIN PASSWORD 'chill_runtime_local_only' IN ROLE chill_app"

python3 - "$BACKEND/testdata/e2e-bootstrap.json" \
  "$ROOT/schemas/behavior/v1/envelope.schema.json" "$BOOTSTRAP_CONFIG" <<'PY'
import json
from pathlib import Path
import sys

configuration = json.loads(Path(sys.argv[1]).read_text())
configuration["schema"]["definition"] = json.loads(Path(sys.argv[2]).read_text())
Path(sys.argv[3]).write_text(json.dumps(configuration, sort_keys=True))
PY
BOOTSTRAP_JSON="$(cargo run --locked --quiet --bin chillctl -- bootstrap --config "$BOOTSTRAP_CONFIG" 2>/dev/null)"
SDK_KEY="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["sdk_key"])' <<<"$BOOTSTRAP_JSON")"
ORGANIZATION_ID="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["organization_id"])' <<<"$BOOTSTRAP_JSON")"
PROJECT_ID="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["project_id"])' <<<"$BOOTSTRAP_JSON")"
ENVIRONMENT_ID="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["environment_id"])' <<<"$BOOTSTRAP_JSON")"

cargo build --locked --quiet --bin chilld
install -m 0755 target/debug/chilld "$SERVER_BINARY"
CHILL_DATABASE_URL="$CHILL_TEST_RUNTIME_DATABASE_URL" \
  CHILL_LISTEN_ADDRESS='127.0.0.1:4318' \
  CHILL_LIFECYCLE_ENABLED='false' \
  "$SERVER_BINARY" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!
for _ in {1..100}; do
  if curl --fail --silent http://127.0.0.1:4318/readyz >/dev/null; then
    break
  fi
  if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    cat "$SERVER_LOG" >&2
    exit 1
  fi
  sleep 0.1
done
curl --fail --silent http://127.0.0.1:4318/readyz >/dev/null || {
  cat "$SERVER_LOG" >&2
  exit 1
}

cd "$ROOT/sdk/swift"
CHILL_OTLP_ENDPOINT='http://127.0.0.1:4318/v1/logs' \
  CHILL_API_KEY="$SDK_KEY" \
  swift run ChillBackendE2E
cd "$BACKEND"

for _ in {1..100}; do
  NORMALIZED_STATUS="$(docker compose -f "$COMPOSE_FILE" exec -T postgres \
    psql --username chill_admin --dbname chill --tuples-only --no-align \
    --command "SELECT status FROM ingest.inbox ORDER BY id DESC LIMIT 1")"
  if [[ "$NORMALIZED_STATUS" == "completed" || "$NORMALIZED_STATUS" == "dead_letter" ]]; then
    break
  fi
  sleep 0.1
done

docker compose -f "$COMPOSE_FILE" exec -T postgres \
  psql --username chill_admin --dbname chill --set ON_ERROR_STOP=1 --tuples-only \
  --command "SELECT CASE WHEN count(*) = 1 THEN 'swift-e2e-ok' ELSE 'swift-e2e-missing' END FROM ingest.inbox AS inbox JOIN control.organizations AS organization ON organization.id = inbox.organization_id WHERE organization.slug = 'swift-backend-e2e' AND metadata #>> '{schema_refs,0,url}' = 'https://schemas.chill.dev/behavior/v1/envelope.schema.json'" \
  | grep --quiet 'swift-e2e-ok'

CANONICAL_JSON="$(docker compose -f "$COMPOSE_FILE" exec -T postgres \
  psql --username chill_admin --dbname chill --tuples-only --no-align \
  --command "SELECT canonical::text FROM ingest.canonical_envelopes WHERE envelope_kind = 'behavior' ORDER BY id DESC LIMIT 1")"
if [[ -z "$CANONICAL_JSON" ]]; then
  docker compose -f "$COMPOSE_FILE" exec -T postgres \
    psql --username chill_admin --dbname chill --no-align \
    --command "SELECT id, status, last_error_code, last_error_message FROM ingest.inbox ORDER BY id DESC LIMIT 1" >&2
  exit 1
fi
printf '%s\n' "$CANONICAL_JSON" | python3 "$ROOT/scripts/validate-canonical-record.py"

for _ in {1..150}; do
  LAKE_STATUS="$(docker compose -f "$COMPOSE_FILE" exec -T postgres \
    psql --username chill_admin --dbname chill --tuples-only --no-align \
    --command "SELECT batch.status FROM lake.export_batches AS batch JOIN control.organizations AS organization ON organization.id=batch.organization_id WHERE organization.slug='swift-backend-e2e' ORDER BY batch.created_at DESC LIMIT 1")"
  if [[ "$LAKE_STATUS" == "committed" || "$LAKE_STATUS" == "dead_letter" ]]; then
    break
  fi
  sleep 0.1
done
if [[ "$LAKE_STATUS" != "committed" ]]; then
  docker compose -f "$COMPOSE_FILE" exec -T postgres \
    psql --username chill_admin --dbname chill --no-align \
    --command "SELECT batch_id,status,last_error_code,last_error_message FROM lake.export_batches ORDER BY created_at DESC LIMIT 5" >&2
  exit 1
fi
cargo run --locked --quiet --bin chillctl -- verify-lake --maximum-files 1000

python3 - "$QUERY_PLAN" <<'PY'
import json
from pathlib import Path
import sys
import time

now = time.time_ns()
Path(sys.argv[1]).write_text(json.dumps({
    "version": 1,
    "kind": "events",
    "range": {
        "start_unix_nano": now - 24 * 60 * 60 * 1_000_000_000,
        "end_unix_nano": now + 60 * 60 * 1_000_000_000,
    },
    "events": {"filter": {}, "limit": 10},
}))
PY
QUERY_JSON="$(cargo run --locked --quiet --bin chillctl -- query \
  --plan "$QUERY_PLAN" --organization "$ORGANIZATION_ID" \
  --project "$PROJECT_ID" --environment "$ENVIRONMENT_ID" \
  --cache-path "$QUERY_CACHE" 2>/dev/null)"
python3 -c 'import json,sys; result=json.load(sys.stdin); assert len(result["rows"]) == 1; assert result["stats"]["file_count"] == 1' <<<"$QUERY_JSON"

docker compose -f "$COMPOSE_FILE" exec -T postgres \
  psql --username chill_admin --dbname chill --tuples-only --no-align \
  --command "SELECT installation_id FROM ingest.canonical_envelopes WHERE organization_id='$ORGANIZATION_ID' AND envelope_kind='behavior' ORDER BY id DESC LIMIT 1" \
  >"$DELETION_TARGET"
chmod 600 "$DELETION_TARGET"

TENANT_EXPORT="$PRIVACY_EXPORT_DIR/tenant.zip"
SUBJECT_EXPORT="$PRIVACY_EXPORT_DIR/subject.zip"
cargo run --locked --quiet --bin chillctl -- privacy-export \
  --kind tenant --organization "$ORGANIZATION_ID" \
  --requested-by backend-e2e --reason privacy.tenant_export \
  --idempotency-key swift-e2e-tenant-export --output "$TENANT_EXPORT" >/dev/null
cargo run --locked --quiet --bin chillctl -- privacy-export \
  --kind data_subject --organization "$ORGANIZATION_ID" \
  --project "$PROJECT_ID" --environment "$ENVIRONMENT_ID" \
  --target-kind installation_id --target-file "$DELETION_TARGET" \
  --requested-by backend-e2e --reason privacy.subject_export \
  --idempotency-key swift-e2e-subject-export --output "$SUBJECT_EXPORT" >/dev/null
python3 - "$TENANT_EXPORT" "$SUBJECT_EXPORT" <<'PY'
import json
import os
from pathlib import Path
import stat
import sys
import zipfile

for path_value, expected_audit in ((sys.argv[1], True), (sys.argv[2], False)):
    path = Path(path_value)
    assert stat.S_IMODE(path.stat().st_mode) == 0o600
    with zipfile.ZipFile(path) as archive:
        assert set(archive.namelist()) == {
            "manifest.json", "records.ndjson",
            "collection-policies.ndjson", "audit.ndjson",
        }
        manifest = json.loads(archive.read("manifest.json"))
        records = archive.read("records.ndjson").splitlines()
        audits = archive.read("audit.ndjson").splitlines()
        policies = archive.read("collection-policies.ndjson").splitlines()
        assert manifest["record_count"] == 1 and len(records) == 1
        assert len(policies) >= 1
        assert bool(audits) is expected_audit
PY

cargo run --locked --quiet --bin chillctl -- lifecycle-request \
  --kind data_subject --organization "$ORGANIZATION_ID" --project "$PROJECT_ID" \
  --environment "$ENVIRONMENT_ID" --target-kind installation_id \
  --target-file "$DELETION_TARGET" --requested-by backend-e2e \
  --reason privacy.e2e_delete --idempotency-key swift-e2e-delete \
  --cache-path "$QUERY_CACHE" >/dev/null
LIFECYCLE_JSON="$(cargo run --locked --quiet --bin chillctl -- lifecycle-run \
  --schedule-retention=false --maximum-requests 10 --cache-path "$QUERY_CACHE")"
python3 -c 'import json,sys; result=json.load(sys.stdin); assert result["processed"] == 1' <<<"$LIFECYCLE_JSON"
QUERY_AFTER_DELETE="$(cargo run --locked --quiet --bin chillctl -- query \
  --plan "$QUERY_PLAN" --organization "$ORGANIZATION_ID" \
  --project "$PROJECT_ID" --environment "$ENVIRONMENT_ID" \
  --cache-path "$QUERY_CACHE" 2>/dev/null)"
python3 -c 'import json,sys; result=json.load(sys.stdin); assert len(result["rows"]) == 0; assert result["stats"]["file_count"] == 0' <<<"$QUERY_AFTER_DELETE"
if find "$QUERY_CACHE" -name '*.parquet' -print -quit | grep --quiet .; then
  echo 'lifecycle deletion left a Parquet query-cache file behind' >&2
  exit 1
fi

EMPTY_SUBJECT_EXPORT="$PRIVACY_EXPORT_DIR/subject-after-delete.zip"
cargo run --locked --quiet --bin chillctl -- privacy-export \
  --kind data_subject --organization "$ORGANIZATION_ID" \
  --project "$PROJECT_ID" --environment "$ENVIRONMENT_ID" \
  --target-kind installation_id --target-file "$DELETION_TARGET" \
  --requested-by backend-e2e --reason privacy.subject_export \
  --idempotency-key swift-e2e-subject-export-after-delete \
  --output "$EMPTY_SUBJECT_EXPORT" >/dev/null
python3 - "$EMPTY_SUBJECT_EXPORT" <<'PY'
import json
import sys
import zipfile
with zipfile.ZipFile(sys.argv[1]) as archive:
    assert json.loads(archive.read("manifest.json"))["record_count"] == 0
    assert archive.read("records.ndjson") == b""
PY

kill "$SERVER_PID" >/dev/null 2>&1 || true
wait "$SERVER_PID" >/dev/null 2>&1 || true
SERVER_PID=""

docker compose -f "$COMPOSE_FILE" exec -T postgres \
  pg_dump --username chill_admin --format custom --dbname chill >"$DUMP_FILE"
docker compose -f "$COMPOSE_FILE" exec -T postgres \
  createdb --username chill_admin chill_restore
docker compose -f "$COMPOSE_FILE" exec -T postgres \
  pg_restore --username chill_admin --dbname chill_restore --exit-on-error <"$DUMP_FILE"

CHILL_DATABASE_URL='postgres://chill_admin:chill_admin_local_only@127.0.0.1:55432/chill_restore?sslmode=disable' \
  cargo run --locked --quiet --bin chillctl -- verify --minimum-organizations 1
CHILL_DATABASE_URL='postgres://chill_admin:chill_admin_local_only@127.0.0.1:55432/chill_restore?sslmode=disable' \
  cargo run --locked --quiet --bin chillctl -- verify-lake --maximum-files 1000
