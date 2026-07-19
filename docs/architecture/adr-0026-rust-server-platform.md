# ADR-0026: Rust owns every server-side component

- Status: Accepted
- Date: 2026-07-16
- Supersedes: ADR-0015 language selection

## Context

The initial backend vertical slice was implemented as a Go modular monolith.
That work validated the service boundaries, PostgreSQL schema, OTLP behavior,
Parquet publication protocol, query limits, privacy workflows, and single-node
operating model. The platform direction now requires one stronger and simpler
language boundary: client software stays native to its platform, while every
component that runs on Chill-controlled servers is implemented in Rust.

This is a language migration, not permission to change externally observable
behavior. Existing SQL migrations, protocol fixtures, storage formats, tenant
isolation, retry semantics, privacy guarantees, operational limits, and
acceptance tests are the compatibility contract.

## Decision

Rust is the only production implementation language for server-side Chill
components. This includes:

- the `chilld` HTTP and gRPC runtime;
- control-plane, authentication, authorization, and project-console APIs;
- OTLP and replay ingestion plus canonical normalization;
- Parquet publication, object-storage integration, and DuckDB queries;
- retention, compaction, consent, export, deletion, and other workers; and
- operational CLIs, migration runners, and server-specific tooling shipped in
  production images.

Client boundaries do not change. The console remains TypeScript/React, Apple
SDKs remain Swift, Android remains Kotlin, and future browser or client SDKs
remain idiomatic for their target platform. Python may continue to validate
language-neutral fixtures during development and CI, but it is not a
production server runtime.

The Rust backend remains a modular monolith initially. One Cargo workspace
produces `chilld` and `chillctl`; domain crates own control-plane, ingest,
normalization, lake, query, lifecycle, and platform boundaries. PostgreSQL and
S3-compatible object storage remain the stateful dependencies, and DuckDB
remains embedded until evidence supports extraction.

## Migration strategy

Migration proceeded by vertical slice:

1. establish a pinned Cargo workspace, strict lint policy, checksummed SQL
   migration support, health endpoints, and parity harness;
2. port the control plane and console API;
3. port OTLP/replay ingestion and normalization;
4. port lake publication and typed query execution;
5. port lifecycle, privacy, and operational workers; and
6. switch deployment, CI, release tooling, and documentation before removing
   the Go module.

The Go implementation served temporarily as an executable specification and
differential-test oracle. Each package was removed after its Rust replacement
passed the same unit, integration, failure, and performance gates. The final
migration gate now rejects server-side Go source and Go build dependencies.

## Rust engineering constraints

- Stable Rust is pinned in `rust-toolchain.toml`; dependencies are locked.
- Unsafe Rust is forbidden in first-party crates. Native database bindings are
  isolated behind narrow modules and reproducible builds.
- Tokio provides bounded asynchronous execution. Every public request and
  background work class retains explicit admission, memory, time, and retry
  limits.
- SQL remains parameterized, tenant context remains server-derived, and
  PostgreSQL row-level security remains forced defense in depth.
- Released SQL migrations remain byte-for-byte immutable and use the existing
  `chill_schema_migrations` ledger and checksums.
- Errors are typed inside domain crates and gain context only at executable
  boundaries. Credentials, payload content, and tenant data are never logged.
- Clippy warnings, formatting, unit tests, integration tests, and release-mode
  builds are required gates.

## Consequences

- Chill has one server implementation language and one ownership boundary.
- Server-dependent product work now extends the Rust implementation.
- Compile times and some library integration work increase, while runtime
  memory control, type-level invariants, and elimination of garbage-collector
  behavior improve.
- The validated modular-monolith and data architecture are retained, avoiding
  an unnecessary service-topology rewrite during the language migration.

## Verification

CHILL-59 tracked the migration. Completion required parity for the former Go
tests and fixtures, successful PostgreSQL backup/restore and deletion drills,
OTLP interoperability, identical migration checksums and Parquet contracts,
production-shaped performance evidence, Rust-only container builds, and a
repository check proving no server-side Go dependency remains. The retained
release profile is the two-vCPU, 4 GiB deployment with the 10-million-row
event-exploration criterion and the remaining workload bounds originally
specified in ADR-0015; its implementation language assumptions are superseded
by this ADR.
