#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKEND="$ROOT/backend"
COMPOSE_FILE="$BACKEND/compose.control-plane.yml"
BOOTSTRAP_CONFIG="$(mktemp -t chill-rust-bootstrap.XXXXXX.json)"
SERVER_LOG="$(mktemp -t chill-rust-server.XXXXXX.log)"
OBJECT_ROOT="$(mktemp -d -t chill-rust-objects.XXXXXX)"
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
  rm -f "$BOOTSTRAP_CONFIG" "$SERVER_LOG"
  rm -rf "$OBJECT_ROOT"
  docker compose -f "$COMPOSE_FILE" down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker compose -f "$COMPOSE_FILE" up --detach --wait postgres minio
docker compose -f "$COMPOSE_FILE" run --rm minio-init

export CHILL_DATABASE_URL='postgres://chill_admin:chill_admin_local_only@127.0.0.1:55432/chill?sslmode=disable'
export CHILL_TEST_RUNTIME_DATABASE_URL='postgres://chill_runtime:chill_runtime_local_only@127.0.0.1:55432/chill?sslmode=disable'
export CHILL_KEY_PEPPER='XFxcXFxcXFxcXFxcXFxcXFxcXFxcXFxcXFxcXFxcXFw='

cd "$BACKEND"
cargo run --locked --quiet --bin chillctl -- migrate
docker compose -f "$COMPOSE_FILE" exec -T postgres \
  psql --username chill_admin --dbname chill --set ON_ERROR_STOP=1 \
  --command "CREATE ROLE chill_runtime LOGIN PASSWORD 'chill_runtime_local_only' IN ROLE chill_app"

jq --slurpfile schema "$ROOT/schemas/behavior/v1/envelope.schema.json" \
  '.schema.definition = $schema[0]' \
  "$BACKEND/testdata/e2e-bootstrap.json" > "$BOOTSTRAP_CONFIG"
BOOTSTRAP_JSON="$(cargo run --locked --quiet --bin chillctl -- bootstrap --config "$BOOTSTRAP_CONFIG")"
SDK_KEY="$(jq --raw-output '.sdk_key' <<<"$BOOTSTRAP_JSON")"

cargo build --locked --quiet --bin chilld
CHILL_DATABASE_URL="$CHILL_TEST_RUNTIME_DATABASE_URL" \
  CHILL_LISTEN_ADDRESS='127.0.0.1:4318' \
  CHILL_OBJECT_STORE_BACKEND='filesystem' \
  CHILL_OBJECT_STORE_PATH="$OBJECT_ROOT" \
  "$BACKEND/target/debug/chilld" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!
for _ in {1..100}; do
  if curl --fail --silent http://127.0.0.1:4318/readyz >/dev/null; then
    break
  fi
  if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    exit 1
  fi
  sleep 0.1
done
curl --fail --silent http://127.0.0.1:4318/readyz >/dev/null

cd "$ROOT/sdk/swift"
CHILL_OTLP_ENDPOINT='http://127.0.0.1:4318/v1/logs' \
  CHILL_API_KEY="$SDK_KEY" \
  swift run ChillBackendE2E

for _ in {1..100}; do
  NORMALIZED_STATUS="$(docker compose -f "$COMPOSE_FILE" exec -T postgres \
    psql --username chill_admin --dbname chill --tuples-only --no-align \
    --command "SELECT inbox.status FROM ingest.inbox AS inbox JOIN control.organizations AS organization ON organization.id=inbox.organization_id WHERE organization.slug='swift-backend-e2e' ORDER BY inbox.id DESC LIMIT 1")"
  if [[ "$NORMALIZED_STATUS" == "completed" || "$NORMALIZED_STATUS" == "dead_letter" ]]; then
    break
  fi
  sleep 0.1
done
if [[ "$NORMALIZED_STATUS" != "completed" ]]; then
  docker compose -f "$COMPOSE_FILE" exec -T postgres \
    psql --username chill_admin --dbname chill --no-align \
    --command "SELECT id,status,last_error_code,last_error_message FROM ingest.inbox ORDER BY id DESC LIMIT 5" >&2
  exit 1
fi

CANONICAL_JSON="$(docker compose -f "$COMPOSE_FILE" exec -T postgres \
  psql --username chill_admin --dbname chill --tuples-only --no-align \
  --command "SELECT canonical::text FROM ingest.canonical_envelopes WHERE envelope_kind='behavior' ORDER BY id DESC LIMIT 1")"
if [[ -z "$CANONICAL_JSON" ]]; then
  exit 1
fi
printf '%s\n' "$CANONICAL_JSON" | python3 "$ROOT/scripts/validate-canonical-record.py"
for _ in {1..100}; do
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
echo rust-swift-e2e-ok
