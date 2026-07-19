# ADR-0005: OpenTelemetry mapping and propagation

- Status: Accepted
- Date: 2026-07-14
- Scope ticket: CHILL-7
- Reference implementation: `contracts/otel/v1/reference.py`

## Context

Chill is a behavior system and an OpenTelemetry participant. Those are related
jobs, but they are not the same data model. Product sessions can last for
hours, pages can be concurrently presented, actions are instantaneous facts,
activities have duration, replay chunks contain protected binary data, and one
session can contain many unrelated distributed traces.

Treating every behavior record as a span would create invalid parentage and
very long spans. Treating behavior only as span events would lose facts when a
trace is not sampled or a process dies before its span is exported. Inventing a
parallel trace system would conflict with host applications that already use
OpenTelemetry and would force developers to pass identifiers manually.

Chill therefore needs one lossless, portable carrier for canonical behavior,
plus normal OpenTelemetry projections for trace interoperability. The mapping
must retain source and observed time, record identity, context snapshots,
resource and instrumentation identity, W3C trace correlation, and at-least-once
deduplication.

## Decision

### One fact, two views

Every accepted canonical Chill record has one authoritative behavior form. Its
lossless OTLP representation is an OpenTelemetry `LogRecord` with `EventName`.
In addition, time-bearing operations participate in the normal OpenTelemetry
trace model as spans.

The two views have deliberately different identities:

- `record_id` identifies one immutable behavior fact and is the behavior-ingest
  idempotency key;
- `subject_id` identifies the durable Chill lifecycle subject such as a page or
  activity;
- `trace_id` identifies one distributed operation; and
- `span_id` identifies one operation within that trace.

An OTLP event/log is accepted into the behavior lake only when it carries the
complete `chill.*` envelope, including `chill.record.id`. Ordinary third-party
OpenTelemetry telemetry remains trace/log/metric data and is not guessed into
product behavior. Derived span-event mirrors are marked as projections and are
never counted as a second behavior fact.

### Signal mapping

| Chill concept | Lossless OTLP carrier | Trace projection | Metrics |
| --- | --- | --- | --- |
| Session and journey lifecycle | Event `LogRecord` | None; these can contain many traces | Backend-derived counts and durations |
| Page lifecycle and focus/exposure | Event `LogRecord` | None; navigation is concurrent UI state, not span parentage | Backend-derived views and exposure |
| Action | Event `LogRecord` | Optional short `INTERNAL` dispatch span when an activation establishes downstream work | Backend-derived action counts |
| Activity start/end | Two event `LogRecord`s | One normal span from start to terminal end | Backend-derived duration/outcome histograms |
| Domain or lifecycle event | Event `LogRecord` | Correlated to the active span; optional projection-only span event | Backend-derived counts |
| Impression | Event `LogRecord` | Correlated only; no span | Backend-derived exposure metrics |
| Replay metadata | Event `LogRecord` | Correlated only; replay bytes never enter OTLP | Bounded SDK health only |
| HTTP, RPC, database, and messaging operations | Behavior record only when separately declared | Official semantic-convention span from native auto-instrumentation | Normal OTel metrics where available |

No canonical record is represented *only* as a span event in V1. A project may
enable projection-only span events for trace-backend usability, but the default
mobile profile leaves them off to avoid duplicate encoding and allocations.

Product analytics metrics are computed from canonical records in the backend.
SDK metrics are limited to bounded operational health such as queue depth,
dropped-record count, export latency, replay buffer pressure, and exporter
failures. Annotation keys, page instances, actor IDs, URLs, and record IDs are
never metric labels by default.

### Action and activity spans

A committed native action always emits its action event. When it has no active
trace and its callback may establish child work, the UI adapter may start one
short `INTERNAL` span around synchronous callback dispatch. Structured child
work inherits that context. Render, layout, raw input, and cancelled gestures
never create action spans.

An `@Activity` participates in the host application's installed tracer. The
SDK starts its span at function entry and ends it on the same terminal path as
the canonical end record. The executable reference pairs start/end fixtures to
show the exported form; a native implementation does not buffer a completed
pair before starting the span.

Activity spans use:

- the stable activity name as the low-cardinality span name;
- `INTERNAL` span kind for domain work;
- the active OTel context as parent, independently of Chill activity ancestry;
- `OK` for `outcome.status = ok`;
- `ERROR` for `error` and `timeout`;
- `UNSET` for `cancelled`, with the exact Chill outcome retained as an
  attribute; and
- record IDs, activity subject ID, attempt, recursion depth, outcome, and the
  immutable annotation/context snapshot as attributes.

A process death may leave a valid activity start event without an end event and
without an exported span. Ingest does not invent an end time or a successful
span.

An activity declared as domain work remains `INTERNAL`. HTTP, database, RPC,
and messaging adapters create their standard client/server/producer/consumer
spans. A macro wrapping that call may be a meaningful parent domain activity,
but it may not create a second fake network span or overwrite semantic-
convention attributes owned by native instrumentation.

### Lossless OTLP log encoding

The reference encoder emits one record per request for clarity. Production
exporters batch records with the same resource and instrumentation scope.
Canonical behavior uses Chill's durable queue and dedicated OTLP Logs encoder;
it does not pass through a host logger's sampling or default attribute-limit
truncation. Trace projections still use the host OTel provider. The behavior
encoder and collector must accept at least 256 LogRecord attributes so the V1
maximum of 128 declared annotations plus envelope and interoperability fields
round-trips intact. An incomplete or truncated envelope is rejected instead of
silently becoming a partial behavior fact.

- `clock.occurred_at_unix_nano` maps to `timeUnixNano`.
- `clock.observed_at_unix_nano` maps to `observedTimeUnixNano`.
- canonical trace ID, current span ID, and trace flags map to the LogRecord
  trace fields;
- canonical scalar and array values map to OTLP `AnyValue` without stringifying
  their logical type;
- the canonical envelope maps losslessly under stable `chill.*` keys;
- declared annotations map under `chill.annotation.*`;
- compatible standard attributes such as `session.id`, `app.screen.id`,
  `app.screen.name`, and `app.widget.id` accompany, but never replace, the
  canonical keys;
- instrumentation maps to OpenTelemetry instrumentation scope; and
- canonical resource properties map to the OTel resource; canonical source
  identifiers remain lossless record attributes; and compatible source facts
  may also have a standard resource projection when a detector establishes
  that they describe the emitting entity.

OTLP/HTTP JSON uses lower-camel field names, lowercase hexadecimal trace/span
IDs, integer enum values, and decimal strings for 64-bit integer fields. The
checked fixtures are exact payloads, not a second informal JSON format.

`app.widget.click` is emitted only for a compatible primary pointer/touch
activation and carries `app.widget.id`. Other semantic activations retain a
stable Chill event name because keyboard, voice, accessibility, remote, and
system commands must not be mislabeled merely to fit a development-status
semantic convention.

### Resources, tenancy, and schema stability

One SDK joins the application's existing OpenTelemetry trace provider and
propagator. It does not install a competing global provider. Chill resource
detectors add only missing attributes and follow the provider's merge policy.
The durable behavior queue emits standard OTLP Logs directly so trace sampling,
host log filtering, and host attribute limits cannot discard product facts.

Client-supplied tenant fields are routing assertions, never authorization. The
ingest credential determines the authoritative organization, project, and
environment, and a mismatch is rejected. Tenant, actor, session, journey,
installation, page, and replay identifiers are not OTel resource attributes
because they can change within one emitting process and often have high
cardinality.

The Chill contract pins semantic-convention compatibility to version `1.43.0`
for V1 and exports `chill.otel.semconv.version`. Development-status conventions
are reviewed before every pin update. The lossless `chill.*` contract evolves
under its own schema version, so a standard attribute rename cannot corrupt or
silently reinterpret stored behavior.

### In-process context

The native runtime stores the current OpenTelemetry context in the platform's
structured async-local mechanism. Swift uses task-local state and preserves the
current actor/executor through generated macro wrappers. Synchronous callbacks
use a scoped context guard. Detached tasks, persisted jobs, and arbitrary
callbacks are explicit propagation boundaries.

There is no public API for setting, copying, or concatenating raw trace IDs.
Developers declare behavior; the runtime creates or joins context. A future
typed context token may bridge a callback boundary after policy checks, but it
must not expose mutable identifiers.

Session, journey, page, and activity subject IDs never become trace parents.
Starting a new distributed operation inside the same session may create a new
trace. Navigation does not implicitly end or re-parent active work.

### W3C HTTP propagation

For configured first-party origins:

1. a client adapter starts or joins a normal `CLIENT` span;
2. it injects W3C `traceparent` and valid `tracestate` immediately before the
   request is sent;
3. the server validates and extracts the remote context, then creates a normal
   `SERVER` span; and
4. service and job activities inherit that context automatically.

Invalid version-00 IDs, all-zero IDs, malformed flags, duplicate or invalid
`tracestate` members, and values outside configured bounds are rejected. An
invalid incoming parent causes a new trace; it never creates a partially
trusted context.

Mobile and web SDKs inject no trace or baggage headers into third-party origins
unless the project explicitly allowlists that exact origin. Redirect handling
re-evaluates the final destination before forwarding headers. Public servers
treat remote trace IDs and the sampled bit as untrusted correlation and a
sampling hint—not as authentication, tenancy, consent, or a command to retain
sensitive data.

### Baggage is empty by default

W3C baggage is a cross-process propagation mechanism, not a general behavior
annotation channel. Chill never automatically puts any of these values in it:

- tenant, actor, installation, session, journey, page, action, activity, or
  replay IDs;
- arbitrary annotations, user-entered text, URLs, search terms, or UI labels;
  or
- consent state, authorization tokens, or raw business identifiers.

A project can explicitly allowlist a small public routing value, such as an
opaque release channel. V1 permits at most eight entries, 128 UTF-8 bytes per
raw value, and 1,024 bytes for the encoded header. The W3C limits are larger;
these are deliberate product privacy/performance budgets.

Received baggage is untrusted and remains in a separate namespace. It never
becomes an annotation automatically and therefore cannot override an outer
annotation scope. A server policy may map a validated allowlisted key after
authentication; root-authoritative local context still wins collisions.

### Queues, messages, and persisted jobs

The producer injects the message-creation span context into first-party message
metadata. A consumer's process span keeps its ambient execution parent and
adds a link to every distinct message-creation context. Batches and fan-out use
multiple links instead of pretending that one message is the unique parent.

A persisted job follows the same rule: enqueue context is a link, while the
job runner's current operation is the parent. Context headers are data with
bounded retention and trust policy. They are not copied into a developer-
visible job argument or used to establish tenant identity.

### Sampling and export

Behavior retention and trace sampling are separate decisions:

- a required behavior fact can be queued even when its correlated trace is not
  sampled;
- a non-recording span context still propagates valid correlation;
- the trace sampled bit is retained on the behavior record when present;
- expensive trace attributes are computed only when the span is recording;
- sampling a span never decides whether a consented canonical action/event is
  durable; and
- behavior sampling is deterministic under the project policy and reports its
  own bounded diagnostics.

The frugal initial deployment accepts normal OTLP over authenticated HTTP using
protobuf in production. JSON is supported for conformance and debugging. Both
paths normalize to the same canonical record and enforce the same record-ID
idempotency. OTLP retries are at least once; resending a record retains its
original `record_id`.

Replay blobs use a separate encrypted object path. Only their already-redacted
metadata, digest, codec, byte count, time range, and object reference can enter
the behavior log mapping.

## Required invariants

Every SDK and ingest implementation must satisfy:

1. **Lossless behavior:** every supported canonical fixture round-trips through
   its OTLP event/log encoding.
2. **No span-only facts:** canonical behavior is not lost solely because a span
   was unsampled, unfinished, or dropped.
3. **No double counting:** trace projections cannot create a second behavior
   fact.
4. **One provider:** Chill joins the application's OTel provider rather than
   replacing it.
5. **Automatic correlation:** native async, HTTP, service, queue, and job
   adapters propagate context without developer-managed IDs.
6. **Independent identities:** session/page/activity IDs do not become trace or
   span IDs.
7. **Trust-boundary enforcement:** headers are injected only into configured
   destinations and re-evaluated across redirects.
8. **Default-empty baggage:** no behavior or identity field propagates as
   baggage without an explicit bounded allowlist.
9. **Outer-first safety:** received context cannot overwrite authoritative
   local annotations.
10. **Semantic-convention restraint:** native protocol instrumentation owns
    standard spans; Chill does not create duplicate protocol spans.
11. **Independent sampling:** product behavior durability does not depend on
    trace sampling.
12. **Bounded metrics:** high-cardinality context and annotations are not metric
    dimensions by default.
13. **Replay separation:** replay bytes never enter OTLP attributes or logs.
14. **At-least-once idempotency:** an OTLP retry retains the canonical record ID.

The executable mapping, propagation model, and conformance tests live under
`contracts/otel/v1`, `examples/otlp/v1`, and `tests/conformance`.

## Consequences

- Chill interoperates with existing trace backends without turning navigation
  or sessions into misleading spans.
- Canonical behavior remains recoverable and queryable independently of trace
  sampling.
- There is some intentional duplication between lossless event/log attributes
  and trace span attributes; batching and disabled fast paths keep this bounded.
- Projects must configure first-party propagation origins and rare baggage keys
  explicitly.
- Development-status semantic conventions can evolve without breaking the
  stored Chill contract.

## Primary references

- OpenTelemetry, [OTLP specification](https://opentelemetry.io/docs/specs/otlp/)
- OpenTelemetry, [Logs data model](https://opentelemetry.io/docs/specs/otel/logs/data-model/)
- OpenTelemetry, [Trace API](https://opentelemetry.io/docs/specs/otel/trace/api/)
- OpenTelemetry, [Trace SDK](https://opentelemetry.io/docs/specs/otel/trace/sdk/)
- OpenTelemetry, [Semantic conventions](https://opentelemetry.io/docs/specs/semconv/)
- OpenTelemetry, [Application events](https://opentelemetry.io/docs/specs/semconv/app/app-events/)
- OpenTelemetry, [HTTP spans](https://opentelemetry.io/docs/specs/semconv/http/http-spans/)
- OpenTelemetry, [Messaging spans](https://opentelemetry.io/docs/specs/semconv/messaging/messaging-spans/)
- OpenTelemetry, [Baggage API](https://opentelemetry.io/docs/specs/otel/baggage/api/)
- OpenTelemetry, [Context propagators](https://opentelemetry.io/docs/specs/otel/context/api-propagators/)
- W3C, [Trace Context](https://www.w3.org/TR/trace-context/)
- W3C, [Baggage](https://www.w3.org/TR/baggage/)
