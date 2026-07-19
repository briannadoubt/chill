#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKEND="$ROOT/backend"
COMPOSE_FILE="$BACKEND/compose.control-plane.yml"

cleanup() {
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
cargo run --locked --quiet --bin chillctl -- migrate >/dev/null
docker compose -f "$COMPOSE_FILE" exec -T postgres \
  psql --username chill_admin --dbname chill --set ON_ERROR_STOP=1 \
  --command "CREATE ROLE chill_runtime LOGIN PASSWORD 'chill_runtime_local_only' IN ROLE chill_app"
cargo test --locked --workspace -- --include-ignored

echo backend-integration-ok
