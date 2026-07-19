# ADR-0024: Consent, collection policy, export, and deletion are auditable workflows

Status: accepted

## Decision

Local application consent is the authority that grants capture. A remote
collection policy can only narrow that grant: it may disable the entire SDK or
individual capture classes, but it cannot turn on behavior, diagnostics,
performance, or replay that the application has not locally allowed. The Swift
runtime evaluates both sources at emission time, including replay observers, so
an in-process consent withdrawal takes effect without recreating the runtime.

Every emitted record carries the effective local privacy-policy version. If an
active remote policy names a different version, the SDK fails closed until the
application adopts that version. Older remote revisions are ignored so a late
response cannot undo a newer restriction.

## Remote collection policy

Collection policies are immutable version rows scoped to one organization,
project, and environment. Exactly one row is active per environment. A
control-write user may replace the active row through an optimistic revision
check and a bounded idempotency key; the referenced privacy-policy version must
already be active. Changes append an administrative audit record.

An SDK key can read only its environment's current state from
`GET /v1/chill/collection-state`. The response is bounded JSON with `no-store`
cache semantics and a revision ETag. The Swift client permits HTTPS, or explicit
loopback HTTP for tests, and applies a fetched state through the locked runtime.
Fetch failures do not broaden collection.

## Privacy exports

Chill supports tenant exports and data-subject exports for installation,
session, or replay identifiers. A subject identifier enters through a private
file and exists in plaintext only in process memory. PostgreSQL retains its
SHA-256 digest, never the plaintext value.

The exporter runs under PostgreSQL row-level security and reads only committed
lake manifests. It bounds source files, source bytes, decoded records, audit
rows, and artifact bytes; verifies every object digest and byte count; decodes
Parquet through the canonical reader; filters subject rows by exact identity;
and produces stable event-time ordering. The mode-0600 ZIP contains:

- `manifest.json`;
- `records.ndjson`;
- `collection-policies.ndjson`; and
- `audit.ndjson`, populated only for a tenant export.

Chill does not retain a second server-side copy of the archive. The requested,
completed, or failed receipt is durable and audited, including the artifact
digest and bounded counts, but the operator-approved destination owns delivery,
expiry, access control, and destruction. Completion is recorded only after the
create-only private-file delivery boundary succeeds. A completed idempotency key
cannot regenerate an archive because retaining the prior artifact would violate
that boundary; recovery uses a new reviewed request.

## Deletion

The existing lifecycle workflow is the destructive counterpart to export. It
rewrites mixed Parquet objects, deletes raw inbox bytes and matching canonical
rows, removes every object-store version, purges verified query-cache entries,
increments the environment generation, and queues derived data for rebuild.
Receipts retain only the subject digest and deletion evidence.

An archive already delivered outside Chill cannot be recalled by storage
deletion. Operators must include delivered exports, downstream copies, and their
backups in the same fulfillment run and retention policy.

## Verification

The backend acceptance gate fetches live collection state with the bootstrapped
SDK key, applies it to the real Swift runtime, emits a macro-generated event,
normalizes and publishes that event to Parquet, validates tenant and subject
exports, fulfills a subject deletion, and proves a new post-deletion export has
zero matching records. Backup and restore verification includes collection
policies, export receipts, credential secrecy, and lifecycle invariants.
