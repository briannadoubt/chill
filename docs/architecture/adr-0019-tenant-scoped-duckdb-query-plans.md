# ADR-0019: Execute typed tenant plans in a constrained DuckDB engine

- Status: Accepted
- Date: 2026-07-15

## Context

Chill needs interactive event exploration, distributed trace lookup, replay
index lookup, ordered funnels, cohorts, and aggregates over the Parquet lake.
The initial deployment must run beside ingest and background workers on a
small host. It cannot expose arbitrary SQL, allow a query to enumerate an
object prefix, leak one tenant's manifests into another tenant's cache key, or
let DuckDB consume the process without a bound.

DuckDB can read S3 directly through `httpfs`, but doing so would place object
credentials and network access inside the analytical engine. At the initial
scale, the simpler and safer option is to reuse Chill's verified object
boundary and maintain a bounded local working set.

## Decision

### Accept versioned plans, never caller-authored SQL

Query plan version `1` is a closed tagged union with six operations:

- `events`: paged behavior records with identity, session, trace, and exact
  annotation filters;
- `trace`: correlated behavior and OpenTelemetry span rows by trace ID;
- `replay`: replay chunk indices by replay or session ID;
- `funnel`: two to eight ordered behavior steps within a completion window;
- `cohort`: subjects meeting a behavior predicate and minimum event count; and
- `aggregate`: count or unique-subject metrics by bounded time bucket and a
  closed dimension set.

Every plan requires a half-open occurrence-time range. Limits, identifiers,
filter sizes, annotation count, funnel depth, completion window, aggregate
interval, metric, and dimension are validated before manifest resolution.
Caller values and local file paths are bound parameters. Only constants from
closed enums select a fixed SQL template. A plan cannot contain a projection,
function, expression, table, path, URL, statement, or SQL fragment.

The internal command-line surface takes explicit organization, project, and
environment UUIDs plus a plan document. The product API calls the same Rust
query service after user/session authorization; it does not translate a
request into free-form SQL.

### Resolve exact committed manifests in PostgreSQL

The catalog opens a tenant-scoped PostgreSQL transaction and includes the full
trusted scope in every policy and manifest predicate even though forced RLS is
also active. It selects only `committed` lake batches whose file-level
effective-time bounds overlap the plan and whose canonical envelope kind is
needed by that plan. Superseded, pending, leased, dead-letter, out-of-scope,
and unrelated-kind files never reach DuckDB.

The manifest generation is a digest of the active privacy-policy version and
digest plus every ordered batch ID and object digest. The result-cache key
includes that generation, full scope, and canonical plan JSON. New files,
compaction, retention, or a privacy-policy change therefore cause a safe miss
without a distributed invalidation message.

### Materialize a verified, bounded local working set

The query file cache downloads through the same immutable object interface as
the lake verifier. A file is published into the cache only after its byte count
and SHA-256 match the committed ledger. Cache filenames are object digests, not
caller or object-key text. Writes use private temporary files, `fsync`, and an
atomic rename.

The cache tracks references so an executing query's files cannot be evicted.
Unreferenced files are removed least-recently-used when space is needed.
Concurrent loads of one digest coalesce, reservations prevent parallel loads
from oversubscribing capacity, and corrupt persisted files are replaced only
after re-verification. A separate bounded TTL/LRU cache stores serialized
results.

This deliberately downloads a whole Parquet object on a cold miss. It is the
right cost/complexity trade at the initial scale and makes local filesystem and
hosted S3 behavior identical. A profile showing cold object transfer dominates
the latency budget is the trigger to add a credential-scoped `httpfs` reader
behind the same catalog interface, not to change plans or manifests.

### Constrain and harden the embedded engine

The first deployment permits one analytical query at a time. A bounded queue
returns `busy` rather than allowing unbounded goroutines or memory contention.
The application also enforces maximum time range, manifest files, declared
scan bytes, result rows, serialized result bytes, queue duration, and execution
duration before or while consuming results.

DuckDB 1.5.4 through the Rust client runs with:

- 768 MiB buffer-manager memory, two threads, and 512 MiB maximum spill;
- a private temporary directory under the verified cache root;
- only that cache root in `allowed_directories`;
- external access disabled after bundled Parquet and JSON extensions load;
- automatic extension install/load and community extensions disabled;
- internal logging disabled so plans and values are not retained; and
- configuration locked before any query executes.

The engine has one database connection, so those global controls apply to
every plan. Context cancellation interrupts the active DuckDB query. Result
shape is checked against the typed plan and every row is counted and serialized
against the response budget while streaming from the driver.

### Observe bounded metadata, not customer values

Every attempt emits one observation with plan kind, stable outcome class,
cache decision, file count, declared scan bytes, returned rows, queue time,
planning time, materialization time, execution time, and total time. It never
logs SQL, object paths, subject/session/trace IDs, annotations, canonical JSON,
or result values. The observer interface is the seam for the later internal
OpenTelemetry metrics implementation.

## Consequences

- The query engine is immediately useful to the internal platform through
  `chillctl query` and is ready for the product authorization/API layer.
- One long query can delay another for at most the configured queue timeout;
  it cannot consume a second engine allocation. More concurrency requires a
  measured resource review or query-process extraction.
- Result invalidation needs no pub/sub service. A stale result can live only
  for its short TTL and only when scope, policy, and exact manifest generation
  remain unchanged.
- Memory limits do not cover every DuckDB allocation. Result/scan/concurrency
  limits and container resource caps remain necessary defense in depth.
- The cache stores behavior data on local disk with process-user permissions.
  Hosted deployment must place it on encrypted ephemeral storage and delete it
  through the retention workflow implemented by CHILL-26.

## Verification

Tests execute all six plan types against real Parquet in DuckDB, prove caller
values never enter SQL structure, reject ambiguous and unbounded plans, enforce
result caps and cancellation, verify locked configuration and filesystem
denial, exercise file corruption/LRU/reference/capacity behavior, prove
generation-aware result caching and queue backpressure, and query a real
tenant-scoped PostgreSQL manifest under forced RLS.

The reproducible 100,000-row workload on the development host completed event
exploration in roughly 36 ms cold and 33 ms warm; ten warm benchmark iterations
averaged roughly 31.8 ms with about 110 KiB allocated per result. The checked-in
benchmark is not a substitute for the two-vCPU deployment profile or the
10-million-row release criterion retained by ADR-0026, which CHILL-27 keeps as
a deployment acceptance gate.
