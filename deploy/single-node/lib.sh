#!/usr/bin/env bash

set -euo pipefail

DEPLOY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$DEPLOY_ROOT/compose.yml"

load_deploy_environment() {
  if [[ -f "$DEPLOY_ROOT/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$DEPLOY_ROOT/.env"
    set +a
  fi
}

compose() {
  local arguments=(--project-directory "$DEPLOY_ROOT")
  if [[ -f "$DEPLOY_ROOT/.env" ]]; then
    arguments+=(--env-file "$DEPLOY_ROOT/.env")
  fi
  arguments+=(-f "$COMPOSE_FILE")
  docker compose "${arguments[@]}" "$@"
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

verify_checksums() {
  local directory="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$directory" && sha256sum --check checksums.sha256)
  else
    (cd "$directory" && shasum -a 256 --check checksums.sha256)
  fi
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "required command not found: $1" >&2
    exit 1
  }
}
