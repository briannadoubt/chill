#!/usr/bin/env bash

set -euo pipefail

# shellcheck source=lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
load_deploy_environment
require_command docker

backup_root="${CHILL_BACKUP_DIR:-$DEPLOY_ROOT/backups}"
if [[ "$backup_root" != /* ]]; then
  backup_root="$DEPLOY_ROOT/${backup_root#./}"
fi
name="${1:-$(date -u +%Y%m%dT%H%M%SZ)}"
if [[ ! "$name" =~ ^[0-9]{8}T[0-9]{6}Z$ ]]; then
  echo "backup name must be an UTC timestamp like 20260715T060000Z" >&2
  exit 1
fi
destination="$backup_root/$name"
lock="$backup_root/.backup-lock"
umask 077
mkdir -p "$backup_root"
chmod 0700 "$backup_root"
if ! mkdir "$lock" 2>/dev/null; then
  echo "another backup is already running" >&2
  exit 1
fi
restart=false
cleanup() {
  local status=$?
  rmdir "$lock" >/dev/null 2>&1 || true
  if [[ "$restart" == true ]]; then
    compose up -d chilld caddy >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

mkdir "$destination"
chmod 0700 "$destination"
if compose ps --status running --services | grep --quiet '^chilld$'; then
  compose stop chilld >/dev/null
  restart=true
fi

compose exec -T postgres pg_dump --username chill_admin --dbname chill --format custom >"$destination/postgres.dump.tmp"
mv "$destination/postgres.dump.tmp" "$destination/postgres.dump"
BACKUP_NAME="$name" CHILL_BACKUP_DIR="$backup_root" compose run --rm object-backup >/dev/null
compose run --rm chillctl verify --minimum-organizations 0 >"$destination/control-verify.json"

(
  cd "$destination"
  : >checksums.sha256
  while IFS= read -r file; do
    digest="$(sha256_file "$file")"
    printf '%s  %s\n' "$digest" "${file#./}" >>checksums.sha256
  done < <(find . -type f ! -name checksums.sha256 | LC_ALL=C sort)
)
verify_checksums "$destination" >/dev/null

retention_days="${CHILL_BACKUP_RETENTION_DAYS:-7}"
if [[ "$retention_days" =~ ^[1-9][0-9]*$ ]]; then
  find "$backup_root" -mindepth 1 -maxdepth 1 -type d -name '20??????T??????Z' -mtime "+$retention_days" -exec rm -rf {} +
fi

if [[ "$restart" == true ]]; then
  compose up -d chilld caddy >/dev/null
  restart=false
fi
printf '%s\n' "$destination"
