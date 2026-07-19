# ADR-0001: Canonical behavior record model

- Status: Accepted
- Date: 2026-07-14
- Scope ticket: CHILL-3
- Schema: `schemas/behavior/v1/envelope.schema.json`

## Context

Chill needs one behavior model that can be produced idiomatically by Swift,
Android, web, and server instrumentation; correlated with OpenTelemetry; stored
cheaply in Parquet; queried for product analytics; and synchronized with session
replay. It must work during intermittent connectivity and must not use a
distributed trace as a substitute for a user session or journey.

The developer-facing API is declarative. A page, action, activity, or annotation
describes meaning. The SDK observes native framework behavior and creates the
records. There is no public `Tracker.track()` workflow.

## Decision

Chill stores an append-only stream of immutable **behavior records**. Every
record describes one lifecycle fact about a logical subject. Instant actions,
impressions, and events use the record itself as the subject. Replay chunks use
their replay stream as the subject. Interval-shaped concepts use a stable
`subject_id` across `start`, optional `update`, and `end` records.

The canonical envelope is the trusted, post-ingestion representation. Ingest
authenticates the source and stamps `tenant.organization_id`,
`tenant.project_id`, and `tenant.environment_id`; SDK-provided values cannot
choose or overwrite those fields.

### Record kinds

| Kind | Meaning | Typical operations |
| --- | --- | --- |
| `session` | A continuous period of use managed by an SDK timeout and lifecycle policy | `start`, `update`, `end` |
| `journey` | A named logical workflow that may cross pages, traces, sessions, or devices | `start`, `update`, `end` |
| `page` | One visible instance of a structured navigation path | `start`, `update`, `end` |
| `impression` | An element satisfied a declared visibility policy | `instant` |
| `action` | A native activation of a semantically named element | `instant` |
| `activity` | Timed UI, domain, network, storage, or task work | `start`, `update`, `end` |
| `event` | An instantaneous lifecycle, domain, error, crash, performance, or experiment fact | `instant` |
| `replay` | Metadata for a content-addressed structural replay chunk | `instant` |

Start and end records are retained independently. A start without an end is a
valid incomplete interval, which is important for crashes, termination, and
offline recovery. Derived interval tables may compact matching lifecycle facts,
but the append-only source remains authoritative.

### Correlation domains

Chill intentionally uses different identifiers for different lifetimes:

| Identifier | Lifetime and purpose |
| --- | --- |
| `record_id` | Idempotency key for exactly one immutable record |
| `subject_id` | Groups lifecycle records for one session, journey, page, activity, or replay subject |
| `installation_id` | Random local installation identity; never a hardware identifier |
| `session_id` | Groups a continuous use period |
| `journey_id` | Optional logical workflow correlation, potentially across sessions |
| `replay_id` | Optional sampled replay stream within a session |
| `page.instance_id` | Correlates records with one visible page instance |
| `trace_id` and `span_id` | W3C/OpenTelemetry correlation for one distributed operation |

New time-bearing Chill record, subject, session, page, journey, and replay IDs
use lowercase UUIDv7 strings at the JSON boundary and fixed 16-byte values in
columnar storage. Installation IDs use random UUIDv4 values. Tenant identifiers
are opaque server-issued strings. IDs must never encode a user email, database
key, device serial number, or other sensitive business value.

W3C trace IDs remain 16-byte lowercase hexadecimal values and span IDs remain
8-byte lowercase hexadecimal values. Chill never reuses a trace ID as a session,
journey, page, or replay identifier.

### Time and ordering

Each record contains:

- `occurred_at_unix_nano`: source wall-clock time;
- `observed_at_unix_nano`: time the trusted ingest layer first observed it;
- optional `monotonic_nano`: source monotonic time within one boot;
- optional `boot_id`: random identity for that monotonic clock epoch; and
- `sequence_number`: a monotonically increasing safe integer for one
  installation and boot.

Unsigned 64-bit nanosecond values are decimal strings in JSON so JavaScript
cannot lose precision. Ingestion does not rewrite source occurrence time. It
uses observed time for freshness and partitioning, preserves late arrivals, and
computes clock-skew diagnostics. Within one boot, `(installation_id, boot_id,
sequence_number)` is the authoritative source order. Across devices or boots,
there is no claim of total ordering.

For an `end` record, `duration_nano` is computed from a monotonic clock whenever
available. Wall-clock subtraction is only a fallback and must be marked by a
diagnostic attribute.

### Names, paths, and cardinality

`name` and page path segments are stable, developer-controlled semantic names.
They are lowercase and may contain dots, underscores, or hyphens. They must not
contain database IDs, UUIDs, timestamps, localized labels, user-entered text, or
other unbounded values.

Dynamic values belong in typed annotations, for example:

```swift
CatView(cat: cat)
    .page("cat")
    .annotation("cat.id", value: cat.id)
```

A navigation path is retained as an ordered array such as `['home', 'cats',
'detail']`. `/home/cats/detail` is a display and query projection, not the source
representation. Page ancestry and presentation behavior are specified in
CHILL-5.

### Annotations and resources

V1 annotations accept strings, booleans, finite numbers, or homogeneous arrays
of those primitives. Nulls, byte arrays, and nested maps are not accepted on the
behavior hot path. Keys are lowercase dot-separated namespaces. Each record is
limited to 128 effective annotations, matching the OpenTelemetry default
attribute-count limit. String values and arrays are also bounded by the schema.

The envelope contains only the effective merged annotation map. Annotation-key
classification, propagation, redaction, and schema metadata live in the project
schema registry and are referenced by `privacy.policy_version`. Outer-first
scope resolution and collision diagnostics are specified in CHILL-4.

`resource.attributes` describes the entity producing telemetry, such as the app,
service, deployment, device class, OS, and SDK. It follows the same bounded V1
value representation. Frequently repeated resources should be dictionary-
encoded or normalized during Parquet writing.

### Outcomes

An `outcome` is present when a lifecycle fact has a meaningful result. Its
status is one of `unset`, `ok`, `error`, `cancelled`, or `timeout`. `reason_code`
is stable and low-cardinality. Error messages, response bodies, and arbitrary
exception descriptions are not reason codes and require explicit privacy-aware
attributes or separate diagnostic storage.

### Privacy state

Every canonical record carries the policy version and capture class applied at
the source. Consent and redaction state are explicit. Session replay bytes are
redacted before leaving the device; the canonical behavior envelope stores only
chunk metadata, a SHA-256 digest, byte count, codec, and an object-storage
reference.

### OpenTelemetry relationship

The envelope preserves optional W3C trace context without making the entire
behavior model an OpenTelemetry span. Activities normally map to spans; actions,
impressions, and events may map to span events or OTel event/log records; and
resources and attributes retain compatible scalar types. The exact mapping,
sampling, link, and baggage rules are defined by CHILL-7.

This separation follows the standards:

- OpenTelemetry attributes use non-empty string keys and `AnyValue` values, and
  the default attribute count limit is 128:
  <https://opentelemetry.io/docs/specs/otel/common/>
- OpenTelemetry event/log records distinguish source `Timestamp` from
  `ObservedTimestamp` and can carry trace context:
  <https://opentelemetry.io/docs/specs/otel/logs/data-model/>
- W3C Trace Context defines 16-byte trace IDs and 8-byte parent/span IDs and
  describes a trace as one distributed logical operation:
  <https://www.w3.org/TR/trace-context/>

### Versioning

`schema_version` uses semantic versioning and `schema_url` identifies the exact
contract. A patch may clarify validation without changing accepted data. A minor
version may add optional fields or enum values after the backend is deployed to
accept them. Removing fields, changing meaning, changing requiredness, or
changing a value type requires a new major version.

Canonical schemas are strict (`additionalProperties: false`). Producers must not
send fields that the negotiated ingest version does not understand. Raw rejected
payloads are never silently coerced into canonical records.

### Idempotency and integrity

- `(tenant.organization_id, tenant.project_id, record_id)` is unique.
- Replaying an identical record is a no-op.
- Reusing a `record_id` with different canonical bytes is an integrity error.
- `observed_at_unix_nano` is set once at first successful observation and is
  stable across retries and compaction.
- Replay chunk metadata is valid only when its digest and byte count match the
  separately stored object.

Cross-field invariants that JSON Schema cannot express directly are also part of
the canonical contract:

- observed time is greater than or equal to source occurrence time after any
  documented clock-skew allowance has been applied;
- instant actions, impressions, and events use `record_id` as `subject_id`;
- session, journey, page, and replay subject IDs match their corresponding
  context or payload IDs;
- page context and page payload describe the same instance and path;
- replay chunk end time is not earlier than its start time; and
- `duration_nano` is emitted only for an `end` record.

`scripts/validate-contract-fixtures.py` enforces these invariants in addition to
Draft 2020-12 schema validation.

## Rejected alternatives

### One trace per user session

Rejected because sessions are long-lived analytical groupings, while a trace
represents one distributed logical operation and is subject to independent
sampling and span lifetime rules.

### Flattened page strings

Rejected because flattened strings lose navigation structure, make tab and
presentation semantics ambiguous, and force parsing during every analysis.

### Arbitrary JSON annotations

Rejected for V1 because nested unbounded objects undermine SDK performance,
privacy classification, schema governance, columnar storage, and predictable
querying. Rich diagnostic bodies can use a separate governed signal path.

### Last-write-wins annotations

Rejected by product decision. Chill is root-authoritative: an established outer
value cannot be overwritten by an inner scope.

## Consequences

- SDKs and storage must support lifecycle reconstruction and incomplete spans.
- Producers need a UUIDv7 generator and a per-boot sequence counter.
- The ingest API and canonical storage schema are distinct artifacts.
- Schema and privacy registries are prerequisites for accepting custom keys at
  external SaaS scale.
- CHILL-4 through CHILL-9 refine precedence, navigation, declarative macros,
  OTel mapping, budgets, and cross-platform fixtures without changing these
  correlation domains.
