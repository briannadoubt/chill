# ADR-0017: canonical normalization preserves source order and trusted time

## Status

Accepted.

## Decision

`chilld` runs a bounded background normalizer over the durable PostgreSQL
inbox. Workers lease rows with `FOR UPDATE SKIP LOCKED`; expired leases are
recoverable, decoding happens outside transactions, and canonical inserts plus
inbox completion commit atomically. Terminal decode/schema errors become
inspectable dead letters. Transient failures retry with bounded exponential
backoff and a maximum attempt count.

Behavior records are recognized only by the versioned `chill.schema.*`
contract. Ordinary OpenTelemetry logs, spans, and metrics remain ordinary OTel
canonical envelopes. Tenant identity is always replaced with the authenticated
organization/project/environment tuple. Canonical behavior JSON is validated
against the active project schema using JSON Schema 2020-12 and its semantic
cross-field invariants before it can be stored.

## Identity and idempotency

Apple exporters persist one random lowercase UUIDv4 installation ID beside the
offline queue. A process UUIDv4 rotates for each pipeline instance. UUIDv7
record and session identities are generated on the device. Instant actions,
impressions, and events use their immutable record ID as subject ID.

The canonical staging table is unique by environment, envelope kind, and
record ID. Replaying the same identity and SHA-256 immutable source content is a
no-op; reusing an identity for different content is an integrity failure. Inbox
request idempotency remains an earlier, independent protection.

Replay uses three separate identities: session UUIDv7, stable replay UUIDv7,
and chunk UUIDv7. Chunks also carry a monotonic chunk index. The encrypted
replay request requires `Chill-Replay-ID`, `Chill-Replay-Chunk-ID`, and
`Chill-Replay-Session-ID`; optional `traceparent` supplies trace/span
correlation without reusing either as a behavior identity.

## Time and ordering

The normalizer stores all of these independently:

- client `occurred_at_unix_nano`;
- client OTLP `observedTimeUnixNano` for diagnostics;
- immutable server receipt time;
- client monotonic time and UUIDv4 boot epoch;
- safe-integer sequence number; and
- effective ordering time, timing class, and signed skew.

Within one installation and boot, `(installation_id, boot_id,
sequence_number, record_id)` is authoritative even when offline delivery is out
of order. Across boots or installations Chill makes no total-order claim.

The initial timing policy is deterministic:

- source time absent: `missing_source_time`, order by receipt;
- more than five minutes in the future: `future_clock_skew`, order by receipt;
- more than 24 hours old: `late`, preserve and order by source time;
- otherwise: `on_time`, order by source time.

Source occurrence is never rewritten. Canonical observed time is
`max(server receipt, source occurrence)` so the canonical invariant remains
true during diagnosed future-clock skew; the exact receipt is retained in its
own trusted column.

## Consequences

- PostgreSQL is durable normalization staging, not the analytics lake.
- Schema validation and retry state are visible before Parquet publication.
- Late and skewed data remain queryable without pretending arrival order is
  user order.
- JSON Schema regular expressions use an ECMAScript-compatible bounded engine
  because the contract intentionally uses negative lookahead for nonzero W3C
  identifiers.
