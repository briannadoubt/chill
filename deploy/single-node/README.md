# Chill single-node deployment

This deployment runs the complete initial Chill data plane on one Docker host:
PostgreSQL control and ingestion state, MinIO object storage, immutable Parquet
publication, DuckDB queries, lifecycle jobs, and a TLS Caddy edge. Only Caddy
publishes host ports. PostgreSQL, MinIO, the Chill process, and private metrics
remain on container networks.

Use a Linux host with at least 2 vCPUs, 4 GiB of RAM, and durable local SSD
storage. The checked limits keep the steady-state stack below that budget;
operator query jobs are separately capped and should be run one at a time on a
host this small. Docker Engine 27 or newer with Compose v2 is recommended.

## First deployment

From this directory:

```sh
./init-secrets.sh
docker compose --env-file .env -f compose.yml up -d --build
./smoke-test.sh
```

`init-secrets.sh` creates mode-0600 secret files and refuses to replace an
existing secret set. Keep `.secrets/` out of source control and copy it to
separate encrypted backup storage. In particular, losing `key_pepper` makes
existing SDK keys unusable. The generated PostgreSQL and MinIO passwords should
also be retained for fresh-host recovery.

Create the first organization by mounting the canonical schema into the admin
container. The SDK key is printed exactly once:

```sh
docker compose --env-file .env -f compose.yml run --rm \
  -v ../../schemas/behavior/v1/envelope.schema.json:/config/envelope.schema.json:ro \
  chillctl bootstrap \
  --owner-email owner@example.com \
  --schema /config/envelope.schema.json
```

The public OTLP endpoint is `https://<CHILL_DOMAIN>/v1/logs` for logs/events,
with corresponding `/v1/traces`, `/v1/metrics`, and `/v1/chill/replay` paths.
The one-time SDK credential is supplied in the authentication header defined by
the ingestion contract.

For a separately hosted console, an operator can issue a short-lived user
session for the identity mapping created by bootstrap. This is an
administrative handoff, not a public login endpoint: verify the operator's
identity outside Chill before adding the acknowledgement flag. The raw
credential is printed exactly once:

```sh
docker compose --env-file .env -f compose.yml run --rm chillctl \
  issue-verified-session \
  --organization <organization-id-from-bootstrap> \
  --issuer chill.email \
  --subject owner@example.com \
  --lifetime-minutes 480 \
  --identity-verified
```

Paste the resulting `ch_us_` credential and `https://<CHILL_DOMAIN>` API origin
into the console sign-in screen. The console keeps the credential in browser
session storage only. Set `CHILL_CONSOLE_ORIGIN` to the exact console origin so
the Rust API accepts browser requests from that one origin.

## Local TLS and a public domain

The default `.env` binds `127.0.0.1:8080` and `127.0.0.1:8443`. Caddy creates a
local CA certificate, so the smoke command intentionally uses the local-only
trust bypass. Production SDKs must never disable certificate verification.

For a public host, point a DNS record at it and set:

```dotenv
CHILL_DOMAIN=telemetry.example.com
CHILL_HTTP_BIND=0.0.0.0
CHILL_HTTP_PORT=80
CHILL_HTTPS_BIND=0.0.0.0
CHILL_HTTPS_PORT=443
CHILL_CONSOLE_ORIGIN=https://chill-console.example.com
```

Allow inbound TCP 80 and 443. Caddy obtains and renews the public certificate.
Do not publish ports 4318, 5432, or 9000 in `compose.yml`.

## Observe and operate

Readiness and build identity are safe at the edge:

```sh
curl https://telemetry.example.com/readyz
curl https://telemetry.example.com/version
docker compose --env-file .env -f compose.yml logs --since 15m chilld caddy
```

Prometheus metrics are deliberately private. Scrape them from the host or an
agent on the data network:

```sh
docker compose --env-file .env -f compose.yml exec -T chilld \
  chillctl metrics
```

The edge returns 404 for `/metrics`. Process and PostgreSQL
pool gauges are emitted in Prometheus text; application and access logs are
structured JSON. `X-Chill-Version` is present on service responses.

Run bounded administration through the non-root, read-only ops container:

```sh
docker compose --env-file .env -f compose.yml run --rm chillctl verify
docker compose --env-file .env -f compose.yml run --rm chillctl verify-lake --maximum-files 1000
```

The runtime owns normalization, lake publication, compaction, and lifecycle
jobs. Keep `CHILL_LIFECYCLE_ENABLED=true` unless exactly one external worker has
been assigned that lease contract.

## Backup and recover

Create a consistent timestamped backup:

```sh
./backup.sh
```

The script briefly stops `chilld`, writes a custom-format PostgreSQL dump,
mirrors object storage, records a control-plane verification receipt, checksums
the complete backup, and restarts the service. Backups default to `./backups`
with seven-day local retention. Copy completed directories and `.secrets/` to
encrypted off-host storage; local retention is not disaster recovery.

Prove a backup without touching production state:

```sh
./restore-drill.sh /absolute/path/to/backups/20260715T120001Z
```

The drill restores PostgreSQL into an isolated database, checks migration and
table invariants, restores objects into an isolated bucket, requires a clean
name/size/time diff, then removes both drill targets.

Restore the live stack only during an incident or an explicit exercise:

```sh
CHILL_RESTORE_CONFIRM=restore-chill \
  ./restore.sh /absolute/path/to/backups/20260715T120001Z
```

This stops the edge and application, replaces the database and object bucket,
reapplies idempotent migrations and the runtime role, clears the verified query
cache, restarts the stack, and runs the smoke gate. For a new host, restore the
encrypted `.secrets/` copy before starting Compose.

## Upgrade and rollback boundary

Edit only the digest-pinned image references in `.env` after reviewing release
notes, then run:

```sh
./upgrade.sh
```

The upgrade creates a backup first, rebuilds Chill with build identity, reapplies
idempotent migrations, recreates the application and edge, waits for TLS
readiness, and runs the smoke gate. Database migrations must remain backward
compatible with the immediately previous application image; the pre-upgrade
backup is the disaster-recovery boundary if that contract is ever violated.

## Repeat the deployment acceptance gate

With generated secrets present, run:

```sh
../../scripts/test-single-node-deployment.sh
```

The gate validates Compose and shell syntax, runs the Rust workspace tests, starts the digest-
pinned stack, proves the security/resource boundary, exercises idempotent
migration, creates a backup, and performs an isolated restore drill. It preserves
the running stack and the generated backup for operator inspection.
