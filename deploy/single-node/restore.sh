#!/usr/bin/env bash

set -euo pipefail

# shellcheck source=lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
load_deploy_environment

if [[ "${CHILL_RESTORE_CONFIRM:-}" != "restore-chill" ]]; then
  echo "destructive restore requires CHILL_RESTORE_CONFIRM=restore-chill" >&2
  exit 1
fi
backup="${1:-}"
if [[ -z "$backup" || ! -f "$backup/postgres.dump" || ! -f "$backup/checksums.sha256" ]]; then
  echo "usage: CHILL_RESTORE_CONFIRM=restore-chill $0 /absolute/backup/directory" >&2
  exit 1
fi
verify_checksums "$backup" >/dev/null
backup_root="$(cd "$(dirname "$backup")" && pwd)"
name="$(basename "$backup")"

compose stop caddy chilld >/dev/null 2>&1 || true
compose exec -T postgres psql --username chill_admin --dbname postgres --set ON_ERROR_STOP=1 --command "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname='chill' AND pid <> pg_backend_pid()" >/dev/null
compose exec -T postgres dropdb --username chill_admin --if-exists chill
compose exec -T postgres createdb --username chill_admin chill
compose exec -T postgres psql --username chill_admin --dbname chill --set ON_ERROR_STOP=1 --command "DO \$\$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='chill_app') THEN CREATE ROLE chill_app NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT; END IF; END \$\$" >/dev/null
compose exec -T postgres pg_restore --username chill_admin --dbname chill --exit-on-error <"$backup/postgres.dump"
compose run --rm migrate >/dev/null
BACKUP_NAME="$name" CHILL_BACKUP_DIR="$backup_root" RESTORE_BUCKET=chill CLEAR_RESTORE_BUCKET=true compose run --rm object-restore >/dev/null
compose run --rm storage-init >/dev/null
compose up -d chilld caddy >/dev/null
"$DEPLOY_ROOT/smoke-test.sh"
