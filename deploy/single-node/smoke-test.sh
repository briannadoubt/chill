#!/usr/bin/env bash

set -euo pipefail

# shellcheck source=lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
load_deploy_environment
require_command curl

port="${CHILL_HTTPS_PORT:-8443}"
url="${CHILL_SMOKE_URL:-https://localhost:${port}}"

ready=false
for attempt in {1..30}; do
  if curl --fail --silent --insecure "$url/readyz" >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "$ready" != true ]]; then
  echo "Chill edge did not become ready at $url within 30 seconds" >&2
  exit 1
fi
curl --fail --silent --show-error --insecure "$url/version" | grep --quiet '"version"'
compose exec -T chilld chillctl metrics | grep --quiet '^chill_up 1$'
compose run --rm chillctl verify --minimum-organizations 0 >/dev/null

published_ports() {
  local container
  container="$(compose ps --quiet "$1")"
  docker inspect --format '{{range $port, $bindings := .NetworkSettings.Ports}}{{if $bindings}}{{$port}} {{end}}{{end}}' "$container"
}

if published_ports postgres | grep --quiet .; then
  echo "PostgreSQL unexpectedly publishes a host port" >&2
  exit 1
fi
if published_ports minio | grep --quiet .; then
  echo "object storage unexpectedly publishes a host port" >&2
  exit 1
fi

echo "single-node-smoke-ok: $url"
