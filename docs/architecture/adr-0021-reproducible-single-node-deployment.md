# ADR-0021: Start with one digest-pinned, recoverable node

## Status

Accepted — 2026-07-15

## Context

Chill needs the reliability boundary of a serious behavior platform before its
traffic justifies a distributed production fleet. The first operator is one
developer on a budget. Requiring Kubernetes, a streaming cluster, a hosted data
warehouse, or a separate worker fleet would make the initial system harder to
understand, pay for, and recover than its workload warrants.

The modular monolith, PostgreSQL leases, immutable object contracts, and typed
DuckDB query plans already make process placement replaceable. The deployment
therefore needs to prove one inexpensive host without making the single host part
of the data model.

## Decision

### Package the complete slice in Docker Compose

One Compose project runs PostgreSQL 18, S3-compatible MinIO, `chilld`, and Caddy.
Short-lived containers initialize storage, migrate and provision database roles,
run typed administration, and copy backups. The steady-state application owns
normalization, Parquet publication, compaction, and lifecycle leases. Those jobs
can move to another identical process later without changing their durable
contracts.

Only Caddy joins the host network boundary. PostgreSQL and MinIO use an internal
data network and publish no host ports. Caddy terminates public TLS, supports HTTP
1 and HTTP/2 to SDKs, proxies h2c internally, caps request bodies, strips its
server identity, adds conservative response headers, and returns 404 for the
private metrics path.

### Make images and build identity reproducible

Every third-party image uses a version tag plus a multi-architecture manifest
digest. The Rust builder and Debian runtime base are also digest pinned. The
application image is built with trimmed paths, stripped binaries, no VCS-derived
implicit metadata, and explicit version, commit, and build time values. `/version`,
`X-Chill-Version`, and JSON startup logs expose that identity.

The image contains only the two Rust binaries (`chilld` and `chillctl`) and their runtime libraries. It runs
as UID/GID 65532 with no login shell. Application, migration, edge, and operator
containers are read-only where their function permits, use bounded tmpfs mounts,
drop privilege escalation, and have memory, CPU, process, and stop-time caps.

### Keep secrets out of environment values and images

Compose mounts mode-0600 secret files. `chilld` and `chillctl` accept mutually
exclusive `NAME` or `NAME_FILE` sources through one bounded, regular-file reader.
Database migration connects as the initialization administrator, creates or
rotates a dedicated `chill_runtime` login, and grants inheritable but
non-switchable membership in the non-login `chill_app` role. It also applies
statement and idle-transaction limits. The long-running process never receives
the administration connection.

MinIO credentials are exposed to the application through a read-only AWS shared
credentials file. The key digest pepper is separately mounted. Generated secrets
are ignored by Git and must be copied to encrypted off-host storage because a
database dump cannot reconstruct SDK-key verification material.

### Treat observability as a private operator interface

The edge exposes liveness, readiness, and build version. Prometheus text remains
private and reports service availability and PostgreSQL connectivity, and
PostgreSQL pool state. Chill and Caddy logs are JSON. A smoke check proves edge
TLS, version identity, private metrics, database invariants, and the absence of
PostgreSQL or MinIO host bindings.

### Define backup, restore, and upgrade as executable workflows

A backup pauses the application writer, creates a custom PostgreSQL archive,
mirrors the object bucket, stores a control-plane verification receipt, and
SHA-256 checksums every artifact before resuming service. A lock prevents two
backups from targeting the same state. Completed local backups expire after a
bounded period and must be replicated off host.

The non-destructive restore drill creates a separate database and bucket, restores
both sides, checks migration/table invariants and an empty object diff, and removes
the drill state. The destructive workflow requires an exact confirmation value,
replaces the live database and bucket, reruns role provisioning and migrations,
clears the local query cache, restarts, and executes the smoke gate.

An upgrade always backs up first. It pulls/rebuilds the selected image, runs
idempotent migrations, recreates only the application and edge, waits for TLS
readiness, and runs the same smoke gate. Migrations remain compatible with the
immediately previous application release so recreating the prior image is a valid
application rollback; the backup is the state rollback boundary.

## Consequences

- The first production availability boundary is one host. Off-host backups and a
  rehearsed replacement-host procedure reduce recovery time but do not provide
  high availability.
- Four GiB and two vCPUs are the documented minimum. Large internal queries run
  sequentially at this size; typed query and container caps fail closed rather
  than swapping the host into failure.
- MinIO is an S3-compatible packaging choice, not a storage dependency. Hosted S3
  can replace it by configuration while preserving immutable object keys and
  manifests.
- Compose secrets are file mounts rather than an external secret manager. The
  `NAME_FILE` boundary lets a later orchestrator or secret agent supply the same
  interface.
- Prometheus collection is host-local initially. A later collector can join the
  private network without publishing the application listener.
- Scaling out is an operational change: external PostgreSQL/object storage, more
  `chilld` replicas, and separately placed leased workers use the existing data
  contracts.

## Verification

The acceptance gate runs the Rust workspace tests, validates the rendered Compose model and all
operator shell scripts, builds the pinned application image, waits for all health
dependencies, proves public/private endpoint and port boundaries, checks non-root
and resource limits, reruns migration provisioning, creates a checksummed backup,
and restores it into isolated database and bucket targets.

The implementation was additionally exercised with a canonical-schema tenant
bootstrap, a destructive live restore retaining that tenant and SDK-key record,
and a full backup-first upgrade followed by the TLS-aware smoke gate.
