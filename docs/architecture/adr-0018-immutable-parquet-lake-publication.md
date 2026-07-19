# ADR-0018: Publish immutable Parquet through a transactional manifest ledger

- Status: Accepted
- Date: 2026-07-15

## Context

Canonical envelopes are durable in PostgreSQL after normalization, but they
are not yet in a cheap analytical format. Chill needs a lake layout that is
queryable by embedded DuckDB, recoverable across process or object-store
failures, isolated by tenant, and affordable at nearly zero initial traffic.

Adding Kafka, a table-format catalog, or a separate compaction service would
raise the operating floor before load justifies it. Directly listing an object
prefix is also unacceptable: listings are eventually consistent on some
compatible stores and cannot distinguish a fully published file from an
orphan left by a crashed writer.

## Decision

### Use a PostgreSQL outbox boundary and immutable objects

`lake.export_batches` is the publication ledger. A worker uses tenant-scoped
row leases to claim canonical records, records their immutable membership in
`lake.batch_records`, and writes a deterministic Parquet object followed by a
deterministic JSON manifest. Only then does one PostgreSQL transaction mark
the batch `committed`.

Readers select committed ledger rows under authenticated tenant scope and use
their explicit object keys. They never discover analytical data by listing a
bucket. A future durable log can replace the claim side of this boundary
without changing canonical records, lake rows, manifests, or query planning.

The cross-system publish sequence is deliberately asymmetric:

1. create or recover a deterministic leased batch in PostgreSQL;
2. encode the same ordered rows using the pinned writer format;
3. create the immutable Parquet key if absent, or verify its digest;
4. create the immutable manifest key if absent, or verify its digest; and
5. transactionally commit both digests and keys in PostgreSQL.

A crash before step 5 can leave unreachable objects but cannot expose an
incomplete dataset. The next lease reconstructs the same bytes and completes
the same publish. An immutable-key collision with a different digest is a
terminal integrity failure, not a retryable overwrite.

### Keep one explicit, stable Parquet schema

Dataset schema `1` contains trusted organization, project, environment, and
data-source scope; envelope and record identity; installation, session,
replay, trace, and span correlation; occurrence, observation, receipt,
effective, monotonic, boot, sequence, skew, and late-arrival fields; and the
lossless canonical JSON object. Common dimensions are dictionary encoded and
the file uses Zstandard compression with one encoding worker.

Rows are sorted by canonical-envelope identity and assigned deterministic
ordinals. The batch digest incorporates the dataset version, pinned writer
format, partition identity, ordered record digests, and compaction lineage.
This prevents a library or format change from silently reusing an old key.

Files use trusted server receipt time for bounded Hive-style partitions:

```text
v1/
  organization=<uuid>/
  project=<uuid>/
  environment=<uuid>/
  day=<yyyy-mm-dd>/
  hour=<hh>/
  kind=<canonical-kind>/
  part-<batch-sha256>.parquet
  manifest-<batch-sha256>.json
```

Client occurrence time remains a column for behavior analysis, but cannot
force writes into arbitrary historical or future partitions.

### Bound micro-batches and compact by replacement

The first deployment claims at most 2,048 rows and approximately 32 MiB of
canonical data per micro-batch. These limits prevent a single worker from
turning accepted maximum-size records into an unbounded allocation.

When four or more committed files share a tenant/hour/kind partition, a
compaction lease writes one replacement file. Its manifest lists the exact
source batch IDs. The final PostgreSQL transaction commits the replacement and
marks every input `superseded` together. Query planning therefore observes
either the inputs or the replacement, never both and never neither. Physical
source deletion is a later retention job; compaction itself is non-destructive.

Transient publication failures use bounded exponential retry. Exhausted
retries and deterministic encoding/integrity errors become `dead_letter`
batches while their canonical source claims and PostgreSQL records remain
available for inspection and an explicit future retry workflow.

### Support local files and S3-compatible storage behind one contract

The object boundary has only immutable put-if-absent and get operations.
Filesystem publication uses a same-directory temporary file, `fsync`, and an
atomic hard-link create. The S3 implementation uses AWS Signature V4,
conditional `PutObject`, and a SHA-256 metadata value checked on every
recovery. It works with hosted S3 and path-style compatible endpoints.

The checked-in MinIO composition is a loopback-only integration fixture with
throwaway credentials. It is not a recommended production deployment. Hosted
deployments provide their own supported S3 endpoint, bucket, credentials, TLS,
versioning, lifecycle, and durability policy.

## Consequences

- PostgreSQL temporarily owns both canonical rows and publication state. This
  is inexpensive and observable at small scale, but its backlog and write
  amplification are explicit extraction signals.
- Orphaned immutable objects are possible and safe. A later sweep may delete
  keys absent from all live and superseded ledger rows after a safety delay.
- The first schema retains canonical JSON in addition to typed columns. This
  costs storage but protects forward compatibility while product queries and
  schema evolution mature.
- Compaction temporarily duplicates object bytes by design. Query correctness
  does not depend on immediate deletion.
- `chillctl verify-lake` downloads every selected committed object and manifest,
  verifies ledger digests and counts, and decodes the Parquet schema. Operators
  can run it after recovery or before a release with an explicit file cap.

## Verification

The release gate covers deterministic encode/decode, immutable conflict
handling, unsafe-path rejection, local filesystem publication, a real
S3-compatible endpoint, an injected failure between object and manifest
writes, lease recovery, repeated compaction, cross-tenant RLS isolation, live
Swift-macro ingestion through committed Parquet, and database backup/restore.
