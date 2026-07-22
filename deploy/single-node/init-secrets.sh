#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SECRETS="$ROOT/.secrets"
umask 077

command -v openssl >/dev/null 2>&1 || {
  echo "openssl is required to generate deployment secrets" >&2
  exit 1
}

mkdir -p "$SECRETS"
chmod 0700 "$SECRETS"
if find "$SECRETS" -type f -size +0c -print -quit | grep --quiet .; then
  echo "$SECRETS already contains secrets; refusing to overwrite them" >&2
  exit 1
fi

postgres_password="$(openssl rand -hex 32)"
runtime_password="$(openssl rand -hex 32)"
minio_user="chill$(openssl rand -hex 8)"
minio_password="$(openssl rand -hex 32)"
s3_runtime_user="chillruntime$(openssl rand -hex 8)"
s3_runtime_password="$(openssl rand -hex 32)"
key_pepper="$(openssl rand -base64 48 | tr -d '\n')"

printf '%s\n' "$postgres_password" >"$SECRETS/postgres_password"
printf '%s\n' "$runtime_password" >"$SECRETS/runtime_database_password"
printf '%s\n' "postgres://chill_admin:${postgres_password}@postgres:5432/chill?sslmode=disable" >"$SECRETS/admin_database_url"
printf '%s\n' "postgres://chill_runtime:${runtime_password}@postgres:5432/chill?sslmode=disable" >"$SECRETS/runtime_database_url"
printf '%s\n' "$key_pepper" >"$SECRETS/key_pepper"
printf '%s\n' "$minio_user" >"$SECRETS/minio_user"
printf '%s\n' "$minio_password" >"$SECRETS/minio_password"
printf '%s\n' "$s3_runtime_user" >"$SECRETS/s3_runtime_user"
printf '%s\n' "$s3_runtime_password" >"$SECRETS/s3_runtime_password"
printf '[default]\naws_access_key_id = %s\naws_secret_access_key = %s\n' "$s3_runtime_user" "$s3_runtime_password" >"$SECRETS/s3_credentials"
chmod 0600 "$SECRETS"/*

if [[ ! -f "$ROOT/.env" ]]; then
  cp "$ROOT/.env.example" "$ROOT/.env"
  chmod 0600 "$ROOT/.env"
fi

echo "Generated file-backed secrets in $SECRETS. Back them up separately on encrypted storage."
