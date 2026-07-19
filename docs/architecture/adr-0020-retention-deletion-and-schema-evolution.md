# ADR-0020: Delete through immutable replacement and a durable lifecycle ledger

## Status

Accepted — 2026-07-15

## Context

Chill's source of analytical truth is a set of immutable Parquet objects selected
through PostgreSQL manifests. Immutability makes ingestion, recovery, and cheap
object storage predictable, but a retention or privacy request cannot update one
row in place. Removing only a PostgreSQL index would leave customer data in
Parquet, a local query cache, an S3 version, or the durable ingestion inbox.

The first deployment must remain a single inexpensive Rust process. It still needs
the semantics expected of a serious behavior platform: environment-specific
retention, shorter replay expiry, data-subject erasure, environment and tenant
deletion, resumable work, late-arrival tombstones, compatible schema evolution,
and an auditable proof of completion.

## Decision

### Use one tenant-scoped lifecycle ledger

`lifecycle.deletion_requests` is the durable job and completion receipt. It is
protected by forced PostgreSQL row-level security and has an organization-scoped
idempotency key. Supported requests are:

- `retention`, with the environment's behavior cutoff;
- `replay_expiry`, with its independently shorter replay cutoff;
- `data_subject`, keyed by installation, session, or replay identity;
- `environment`; and
- `tenant`.

Leases, bounded retries, errors, start and completion times, row/file/byte counts,
and a deterministic completion digest live in the request. The data-subject value
is needed only while the job runs. Completion sets it to `NULL`; the ledger keeps
only its digest and bounded operational metadata.

### Stop resurrection before deleting existing data

Creating a data-subject request atomically creates a one-way subject tombstone.
Normalization hashes installation, session, and replay IDs and rejects any record
matching an active tombstone. This also covers an offline SDK retry arriving after
deletion. Normalization independently rejects records older than behavior or replay
retention.

Environment and tenant requests immediately revoke scoped SDK keys and suspend the
scope. Normalization rejects all records while such a deletion is pending or
leased. Completion changes the scope to `deleted`. A failed job remains suspended
and visible to operators instead of silently reopening ingestion.

Completed normalization replaces raw inbox payload bytes with a fixed disposition.
Replay metadata is also reduced to that disposition. Non-replay schema references
remain for operational validation. The original SHA-256 receipt digest remains so
an identical SDK retry can recover its idempotent acknowledgement without restoring
the raw payload. A deletion sanitizes affected mixed inbox rows and removes an inbox
row once no canonical record references it.

### Replace a queryable file before erasing its source

For each affected committed Parquet file, the worker:

1. downloads it and verifies the ledger size and SHA-256 digest;
2. decodes and filters rows using a closed request predicate;
3. writes a deterministic `rewrite` batch when unaffected rows survive;
4. publishes its Parquet object and immutable manifest;
5. in one tenant transaction, registers the replacement, records lineage, changes
   the source from `committed` to `tombstoned`, and queues its object and manifest
   for deletion; and
6. only then removes canonical rows and indexes.

The query catalog only selects `committed` batches. There is therefore no point at
which a query can select both the old source and its replacement, or select neither
after a successful ledger transaction. A fully deleted file needs no replacement.
Superseded compaction inputs containing target data are tombstoned and erased as
well, even though queries no longer select them.

An active lake publication lease makes lifecycle work retry. Once a request exists,
the lake writer will not take another lease or build a new micro-batch in that
scope. Expired or pending publication batches are cancelled, their claims are
released, and unaffected canonical rows can be published after deletion completes.
Objects written by a worker that lost its lease remain unreachable and are safe for
the orphan sweep; they can never enter the committed catalog.

### Treat object storage and caches as part of deletion

The object queue is durable and records both the Parquet and manifest digest. The
S3 adapter enumerates every exact object version and delete marker, deletes them by
version ID, and verifies that none remain. Listing or deleting versions is
fail-closed: a lifecycle receipt cannot complete without the required bucket
permissions. Filesystem deletion is idempotent and synchronizes the containing
directory.

Query cache filenames are content digests. The worker removes every deleted digest
from the configured cache root. It also increments
`lifecycle.environment_generations`; that generation participates in every query
result-cache key, so stale in-memory results become unreachable even across
processes. Cache invalidations are derived from the durable object queue and are
replayed after a crash.

Trace, replay, funnel, cohort, aggregate, and general query derived data receive a
generation-specific rebuild row. Derived datasets are disposable products of the
committed lake and can be rebuilt without customer payloads in the job itself.

### Make schema changes explicit and compatibility checked

`lake.dataset_schemas` records active and draft dataset contracts. The compiled
version-1 Rust row shape is the authority for the current writer. A proposed version
must advance exactly once. Logical types cannot change. Backward compatibility
requires new fields to be optional; forward compatibility preserves every existing
required field; full compatibility enforces both. A proposal is only a draft—code,
reader, and migration review are still required before activation.

### Schedule retention in the frugal process

`chilld` schedules one daily idempotent behavior-retention job and one replay-expiry
job per active environment, then runs the same leased lifecycle manager used by
`chillctl`. A later dedicated worker can use the identical PostgreSQL and object
contracts. `CHILL_LIFECYCLE_ENABLED=false` is available for maintenance or when
another deployment owns the worker.

## Failure and recovery properties

- A crash before replacement registration leaves immutable, unreachable objects;
  retry writes the same bytes and keys.
- A crash after registration sees the source tombstoned and does not rewrite it
  again.
- A crash after physical deletion replays cache invalidation from the durable object
  row and does not double-count bytes or files.
- A storage outage keeps the request pending or eventually failed; the subject
  tombstone and any scope suspension remain active.
- The target predicate is never caller-provided SQL. IDs and cutoffs are values in a
  closed set of typed request kinds.
- Tenant scope is present in application predicates and forced RLS policies.

## Consequences

- A data-subject request may read many files until subject-aware file statistics or
  bloom filters are added. `MaximumFiles` bounds each pass; the initial scale makes
  this acceptable.
- Rewrite temporarily consumes extra object bytes. The old source becomes
  unqueryable before it is erased, and byte/file counts expose cleanup lag.
- SHA-256 subject tombstones are suitable for high-entropy generated identifiers,
  not human emails or phone numbers. Supporting those identifiers requires a
  keyed digest and key-rotation contract before ingestion.
- S3 lifecycle credentials need `ListBucketVersions` plus version-specific delete
  permission. Missing permission correctly blocks completion.
- Derived rebuild consumers are intentionally decoupled; product analytics work can
  consume the durable queue without changing deletion semantics.

## Verification

Automated acceptance covers schema compatibility, request validation, forced RLS,
normalization-time retention and tombstones, mixed-file rewrite, canonical/index
erasure, filesystem and version-aware S3 deletion, disk-cache purge, generation
invalidation, derived rebuild rows, audit receipts, and retry-safe empty query
results. The full backend gate also exports a real Swift macro event, publishes it
to S3-compatible Parquet, queries it through DuckDB, deletes its installation, and
proves the same query returns an empty array before backup and restore.

## References

- [Amazon S3: deleting object versions](https://docs.aws.amazon.com/AmazonS3/latest/userguide/DeletingObjectVersions.html)
- [Amazon S3 `DeleteObject` versioning behavior and permissions](https://docs.aws.amazon.com/AmazonS3/latest/API/API_DeleteObject.html)
