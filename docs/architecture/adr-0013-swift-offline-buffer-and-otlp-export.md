# ADR-0013: Swift offline buffer and OTLP export

- Status: Accepted
- Date: 2026-07-15
- Scope ticket: CHILL-16
- Builds on: ADR-0005, ADR-0006, ADR-0008

## Context

Canonical behavior facts must survive ordinary network loss and process death
without moving file or network work onto a UI callback. Delivery is
at-least-once: a retry must preserve the original `record_id`, and a fact may be
removed only after the receiver acknowledges the request. The first deployment
must remain inexpensive, so one bounded local queue and ordinary OTLP/HTTP are
preferable to a mobile database or another full telemetry SDK.

The queue also needs a deterministic answer under pressure. Replay metadata and
impressions are replaceable; actions, events, lifecycle facts, and activity ends
are not. Auth headers, error bodies, URLs other than the configured collector,
and network payloads must never enter persistence or diagnostics.

## Decision

### One configuration object and one sink

`ChillExport` is an additive library product, re-exported by `Chill`. An
application creates one `ChillOfflinePipeline` during telemetry configuration
and passes it to `ChillRuntime` as its `RecordSink`. This is delivery
configuration, not an event API. Product code still has no `track`, `record`, or
trace-ID call.

`RecordSink.submit` performs only a bounded lock-protected memory admission and
schedules one utility drain. Encoding, file operations, batching, compression,
and network I/O occur behind that boundary. The default memory capacity is
1,024 records. The default disk hard cap is 64 MiB, matching the Apple core
release budget.

### Atomic append-only queue

Every accepted fact becomes one immutable queue file. Its binary envelope
contains a version magic, priority, kind, record ID, payload length, and CRC-32
before the OTLP `LogRecord` protobuf bytes. A write uses an atomic replacement;
on iOS-family systems it also uses complete-until-first-authentication file
protection. Startup scans files in deterministic occurrence/sequence order,
removes duplicate record IDs, quarantines malformed or checksum-invalid files,
and re-applies the configured hard cap. Temporary or corrupt files never become
export candidates.

Capacity eviction is deterministic: replay first, then low-priority
impressions. A high-priority fact never evicts another high-priority fact. If
the hard cap contains only high-priority data, the new fact is rejected and a
bounded priority counter records the condition. Those counters contain no
record IDs, annotation values, or other high-cardinality labels.

Export peeks do not mutate the queue. Failures and process death therefore
produce the same ordered record-ID batch on the next attempt. Only an
acknowledged ID is unlinked. An OTLP 2xx response acknowledges the submitted
batch; retryable HTTP responses, disconnects, and timeouts retain it. A
non-retryable response pauses automatic retry but retains the files for a later
configuration fix or launch.

### OTLP/HTTP protobuf and gzip

The durable payload is the stable binary protobuf form of an OTLP `LogRecord`.
The small encoder implements only the official stable messages needed for
`ExportLogsServiceRequest`: `AnyValue`, `KeyValue`, `Resource`,
`InstrumentationScope`, `LogRecord`, `ScopeLogs`, and `ResourceLogs`. It does
not install a logger provider or pull a second general telemetry SDK into the
app. The wire contract is pinned to [OpenTelemetry Protocol
v1.10.0](https://github.com/open-telemetry/opentelemetry-proto/releases/tag/v1.10.0)
and follows the [OTLP specification](https://opentelemetry.io/docs/specs/otlp/).
A JSON encoder remains for exact conformance and debugging.

One export request shares a resource and instrumentation scope across up to 100
records or 512 KiB by default. The exporter sends
`application/x-protobuf` to the configured OTLP Logs endpoint and uses gzip when
it reduces the body. The endpoint is HTTPS unless the developer explicitly
permits HTTP on loopback for local development. Authentication headers remain
memory-only. The production URLSession waits for connectivity, reuses
connections, and has bounded request/resource timeouts.

Failures use exponential backoff with bounded jitter, honor a bounded
numeric `Retry-After`, and never schedule a request while the queue is empty.
Behavior and replay sampling already occur before this sink, so retry state
cannot change a sampling decision.

### Lossless projection

The LogRecord keeps source and observed Unix time, boot-monotonic time,
sequence, canonical record and subject IDs, immutable annotations, session,
page, element, privacy, payload, and trace correlation. Logical scalar and
array annotation types map to OTLP `AnyValue`; 64-bit values remain integers.
Compatible `session.id`, `app.screen.*`, and `app.widget.id` attributes
accompany but never replace canonical `chill.*` keys. Replay bytes do not enter
OTLP Logs; CHILL-17 owns their separately bounded encrypted/redacted chunk
lifetime.

## Consequences

- A normal crash can leave an unacknowledged fact, but cannot create a partial
  accepted queue record or change its ID.
- Offline delivery is at-least-once and relies on backend `record_id`
  idempotency.
- The hot UI path pays one bounded lock and memory append, not serialization or
  I/O.
- The queue is intentionally a specialized append-only spool, not a queryable
  database.
- Cross-process kill tests and physical-device storage/network budgets remain
  release gates in CHILL-18.
