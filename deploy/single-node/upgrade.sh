#!/usr/bin/env bash

set -euo pipefail

# shellcheck source=lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
load_deploy_environment

"$DEPLOY_ROOT/backup.sh" >/dev/null
export CHILL_COMMIT="${CHILL_COMMIT:-$(git -C "$DEPLOY_ROOT/../.." rev-parse --short=12 HEAD 2>/dev/null || echo unknown)}"
export CHILL_BUILD_TIME="${CHILL_BUILD_TIME:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
compose build --pull chilld
compose run --rm migrate >/dev/null
compose up -d --no-deps --force-recreate chilld
compose up -d --no-deps --force-recreate caddy
"$DEPLOY_ROOT/smoke-test.sh"
