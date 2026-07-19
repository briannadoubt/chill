#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY="$ROOT/deploy/single-node"

if [[ ! -f "$DEPLOY/.env" || ! -f "$DEPLOY/.secrets/admin_database_url" ]]; then
  echo "initialize deploy/single-node with ./init-secrets.sh first" >&2
  exit 1
fi

for script in "$DEPLOY"/*.sh; do
  bash -n "$script"
done

docker compose --project-directory "$DEPLOY" --env-file "$DEPLOY/.env" \
  -f "$DEPLOY/compose.yml" config --quiet

(
  cd "$ROOT/backend"
  cargo test --locked --workspace
)

docker compose --project-directory "$DEPLOY" --env-file "$DEPLOY/.env" \
  -f "$DEPLOY/compose.yml" up -d --build
"$DEPLOY/smoke-test.sh"
if docker compose --project-directory "$DEPLOY" --env-file "$DEPLOY/.env" \
  -f "$DEPLOY/compose.yml" logs --since 30s chilld | grep --quiet '"level":"ERROR"'; then
  echo "chilld emitted an error after becoming ready" >&2
  exit 1
fi

container="$(docker compose --project-directory "$DEPLOY" --env-file "$DEPLOY/.env" \
  -f "$DEPLOY/compose.yml" ps --quiet chilld)"
read -r user memory pids readonly no_new_privileges < <(
  docker inspect --format '{{.Config.User}} {{.HostConfig.Memory}} {{.HostConfig.PidsLimit}} {{.HostConfig.ReadonlyRootfs}} {{json .HostConfig.SecurityOpt}}' "$container"
)
[[ "$user" == "65532:65532" ]]
[[ "$memory" -gt 0 ]]
[[ "$pids" -gt 0 ]]
[[ "$readonly" == "true" ]]
[[ "$no_new_privileges" == *no-new-privileges:true* ]]

docker compose --project-directory "$DEPLOY" --env-file "$DEPLOY/.env" \
  -f "$DEPLOY/compose.yml" run --rm migrate >/dev/null
membership="$(docker compose --project-directory "$DEPLOY" --env-file "$DEPLOY/.env" \
  -f "$DEPLOY/compose.yml" exec -T postgres psql --username chill_admin --dbname chill \
  --tuples-only --no-align --command \
  "SELECT inherit_option AND NOT set_option FROM pg_auth_members WHERE roleid='chill_app'::regrole AND member='chill_runtime'::regrole")"
[[ "$membership" == "t" ]]

backup_name="$(date -u +%Y%m%dT%H%M%SZ)"
backup="$($DEPLOY/backup.sh "$backup_name")"
"$DEPLOY/restore-drill.sh" "$backup"

echo "single-node-deployment-ok: backup=$backup"
