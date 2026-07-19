#!/usr/bin/env bash

set -euo pipefail

# shellcheck source=lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
load_deploy_environment

backup_root="${CHILL_BACKUP_DIR:-$DEPLOY_ROOT/backups}"
if [[ "$backup_root" != /* ]]; then
  backup_root="$DEPLOY_ROOT/${backup_root#./}"
fi
backup="${1:-$(find "$backup_root" -mindepth 1 -maxdepth 1 -type d -name '20??????T??????Z' | LC_ALL=C sort | tail -n 1)}"
if [[ -z "$backup" || ! -f "$backup/postgres.dump" || ! -f "$backup/checksums.sha256" ]]; then
  echo "a valid backup directory is required" >&2
  exit 1
fi
verify_checksums "$backup" >/dev/null

database="chill_restore_$(date -u +%Y%m%d%H%M%S)_$$"
bucket="chill-restore-$(date -u +%Y%m%d%H%M%S)-$$"
cleanup() {
  compose exec -T postgres dropdb --username chill_admin --if-exists "$database" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

compose exec -T postgres createdb --username chill_admin "$database"
compose exec -T postgres pg_restore --username chill_admin --dbname "$database" --exit-on-error <"$backup/postgres.dump"
migrations="$(compose exec -T postgres psql --username chill_admin --dbname "$database" --tuples-only --no-align --command 'SELECT count(*) FROM public.chill_schema_migrations')"
tables="$(compose exec -T postgres psql --username chill_admin --dbname "$database" --tuples-only --no-align --command "SELECT count(*) FROM information_schema.tables WHERE table_schema IN ('control','ingest','lake','lifecycle')")"
if [[ "$migrations" -lt 6 || "$tables" -lt 20 ]]; then
  echo "restored database failed schema invariants: migrations=$migrations tables=$tables" >&2
  exit 1
fi

name="$(basename "$backup")"
BACKUP_NAME="$name" CHILL_BACKUP_DIR="$backup_root" RESTORE_BUCKET="$bucket" DRILL_CLEANUP=true compose run --rm object-restore >/dev/null
printf '{"database":"%s","migration_count":%d,"table_count":%d,"objects_verified":true,"restored":true}\n' "$database" "$migrations" "$tables"
