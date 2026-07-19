# Chill backend

Chill's modular monolith is implemented in Rust under
[ADR-0026](../docs/architecture/adr-0026-rust-server-platform.md). Every
server-side component is Rust; client code remains native to its platform.

The Rust workspace contains checksummed migrations, domain crates, and the
`chilld` and `chillctl` executables:

```sh
cd backend
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run --bin chillctl -- migrate
cargo run --bin chilld
```

The underlying architecture remains a frugal modular monolith backed by
PostgreSQL. The control plane provides checksummed
migrations, tenant-enforced row-level security, an atomic first-tenant
bootstrap, and one-time SDK keys.

## Local validation

The full gate uses the pinned PostgreSQL container, runs unit, lint, and integration
tests, creates a custom-format backup, restores it into a fresh database, and
verifies the restored schema. It also starts `chilld`, exports a macro-produced
event from the real Swift offline pipeline, and checks its schema-valid
canonical behavior row:

```sh
../scripts/test-backend-control-plane.sh
```

For manual use:

```sh
cd backend
export CHILL_DATABASE_URL='postgres://chill_admin:chill_admin_local_only@127.0.0.1:55432/chill?sslmode=disable'
export CHILL_KEY_PEPPER='<base64 encoded 32+ random bytes>'
docker compose -f compose.control-plane.yml up --detach --wait postgres
cargo run --locked --bin chillctl -- migrate
cargo run --locked --bin chillctl -- bootstrap --owner-email owner@example.com
```

`bootstrap` uses a migration-owner/admin connection because the first tenant
does not exist yet. It reveals the SDK key once; only its prefix and an
HMAC-SHA-256 digest are stored. Request paths should use a login role that is a
member of `chill_app`, authenticate the SDK key, and set tenant context through
`Store::begin_tenant` before accessing control-plane data.

Human access uses short-lived opaque user sessions created only after an
identity adapter verifies a provider assertion. The current organization role
is resolved on every session authentication, so suspensions and role changes
take effect without waiting for token expiry. Internal workers use separately
typed `ch_sv_` service credentials bound to one organization, project, and
environment. SDK keys, service credentials, and user sessions support atomic
rotation and immediate database-backed revocation; plaintext credentials are
returned once and never persisted. See
[`ADR-0023`](../docs/architecture/adr-0023-tenant-access-control-and-rbac.md).

## Ingestion server

After migrations and bootstrap, start the runtime with a `chill_app` member
database role:

```sh
export CHILL_DATABASE_URL='postgres://chill_runtime:...@127.0.0.1:55432/chill?sslmode=disable'
cargo run --locked --bin chilld
```

The default `:4318` listener accepts OTLP/HTTP protobuf or JSON at `/v1/logs`,
`/v1/traces`, and `/v1/metrics`, standard OTLP/gRPC over h2c, and encrypted
replay chunks at `/v1/chill/replay`. An SDK key reads its environment's active
remote restriction from `GET /v1/chill/collection-state`; local consent remains
the only source that can grant capture. See
[`ADR-0016`](../docs/architecture/adr-0016-authenticated-durable-otlp-ingestion.md)
for authentication headers, limits, idempotency, and retry semantics.
The same process leases inbox work and performs versioned canonical
normalization as described by
[`ADR-0017`](../docs/architecture/adr-0017-canonical-normalization-ordering-and-time.md).

## Parquet lake

The same process leases normalized records into deterministic, Zstandard
Parquet micro-batches and commits immutable manifests. For a local-only store:

```sh
export CHILL_OBJECT_STORE_BACKEND=filesystem
export CHILL_OBJECT_STORE_PATH="$PWD/.chill-data/objects"
```

For hosted S3 or a compatible endpoint, use the standard AWS credential
environment or shared configuration plus:

```sh
export CHILL_OBJECT_STORE_BACKEND=s3
export CHILL_S3_BUCKET=chill-production
export CHILL_S3_REGION=us-west-2
# Optional for a compatible provider:
export CHILL_S3_ENDPOINT=https://objects.example.com
export CHILL_S3_PATH_STYLE=true
```

After a restore or object-storage incident, verify committed manifests and
Parquet contents with a deliberately bounded scan:

```sh
cargo run --locked --bin chillctl -- verify-lake --maximum-files 1000
```

See
[`ADR-0018`](../docs/architecture/adr-0018-immutable-parquet-lake-publication.md)
for the atomic publication, recovery, partitioning, and compaction contract.

## Internal typed queries

The query module never accepts SQL. Internal operators submit a versioned JSON
plan to `chillctl`; the selected organization/project/environment scope is
enforced in both SQL predicates and PostgreSQL RLS. For example:

```json
{
  "version": 1,
  "kind": "events",
  "range": {
    "start_unix_nano": 1784000000000000000,
    "end_unix_nano": 1784086400000000000
  },
  "events": {
    "filter": {"name": "checkout"},
    "limit": 100
  }
}
```

```sh
cargo run --locked --bin chillctl -- query \
  --plan query.json \
  --organization '<organization-uuid>' \
  --project '<project-uuid>' \
  --environment '<environment-uuid>'
```

The process verifies manifest digests into a bounded local cache, runs only a
closed set of parameterized DuckDB plans, and caps time range, files, scanned
bytes, result rows, serialized bytes, memory, threads, spill, queue time, and
execution time.

## Retention, privacy deletion, and schema evolution

`chilld` schedules environment retention and the shorter replay-retention
policy, then runs the resumable lifecycle worker in the same frugal process.
Set `CHILL_LIFECYCLE_ENABLED=false` only when an operator or a separate worker
owns that job. A data-subject identifier is supplied through a private file so
it does not appear in shell history or the process list:

```sh
chmod 600 subject.txt
cargo run --locked --bin chillctl -- lifecycle-request \
  --kind data_subject \
  --organization '<organization-uuid>' \
  --project '<project-uuid>' \
  --environment '<environment-uuid>' \
  --target-kind installation_id \
  --target-file subject.txt \
  --requested-by privacy@example.com \
  --reason privacy.user_request \
  --idempotency-key request-2026-07-15

cargo run --locked --bin chillctl -- lifecycle-run --schedule-retention=false
```

The worker rewrites a mixed Parquet file before tombstoning its source, removes
canonical rows and raw inbox bytes, deletes every S3 object version, purges
verified query-cache files, increments the environment generation, queues all
derived indexes for rebuild, and records a digest-bound completion receipt.
The plaintext subject is erased from the job; its SHA-256 tombstone remains so
offline retries cannot resurrect the deleted identity.

Export before deletion when policy requires delivering a copy. Tenant exports
include tenant audit history; subject exports include only exact matching
canonical records and applicable collection-policy versions. Supply a subject
identifier through the same private-file boundary:

```sh
cargo run --locked --bin chillctl -- privacy-export \
  --kind data_subject \
  --organization '<organization-uuid>' \
  --project '<project-uuid>' \
  --environment '<environment-uuid>' \
  --target-kind installation_id \
  --target-file subject.txt \
  --requested-by privacy@example.com \
  --reason privacy.user_export \
  --idempotency-key export-2026-07-15 \
  --output subject-export.zip
```

The destination must not exist and is created mode 0600. Chill records the
artifact digest and counts only after delivery succeeds; it does not retain a
server-side archive. The operator remains responsible for expiry and deletion
of the delivered copy. See
[`ADR-0024`](../docs/architecture/adr-0024-consent-export-and-deletion-workflows.md).

Dataset changes are registered as compatibility-checked drafts. Adding an
optional column is accepted; a logical-type change or a new required column is
rejected before a writer version can change:

```sh
cargo run --locked --bin chillctl -- schema-propose \
  --definition dataset-schema-v2.json \
  --compatibility full
```

See
[`ADR-0020`](../docs/architecture/adr-0020-retention-deletion-and-schema-evolution.md)
for ordering, retries, tombstones, cache invalidation, S3 version erasure, and
schema-compatibility rules.

## Single-node production slice

The initial TLS-terminated, digest-pinned deployment packages this service with
PostgreSQL, MinIO, private metrics, file-backed secrets, resource limits, backup,
restore drills, and backup-first upgrades. Follow the
[single-node operator guide](../deploy/single-node/README.md) or run its acceptance
gate after initializing secrets:

```sh
../scripts/test-single-node-deployment.sh
```
