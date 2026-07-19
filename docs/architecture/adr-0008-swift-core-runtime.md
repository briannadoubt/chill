# ADR-0008: The Swift core is typed, immutable, gated, and package-internal

- Status: Accepted
- Date: 2026-07-14
- Owners: Apple SDK architecture
- Decision scope: `sdk/swift` core runtime

## Context

The first complete Chill SDK ships on Apple platforms, but application code
must not imperatively emit telemetry. The core still needs a small, fast seam
for future Swift macros, SwiftUI, UIKit, networking, replay, and delivery
targets. It must implement the shared contract without forcing all of those
features into one binary.

The disabled, denied, and sampled-out paths are especially important. A
declaration that cannot produce a permitted fact must not build annotations,
generate an ID, read a clock, acquire a runtime lock, or touch a sink. This is
both a performance invariant and a privacy invariant.

## Decision

`sdk/swift` is a Swift 6 package with strict concurrency and no third-party
dependencies in `ChillCore`. Its initial deployment floor is iOS/tvOS 17,
watchOS 10, macOS 14, and visionOS 1. Later modules remain separate products so
applications link only the instrumentation they use.

### Public semantic types

The core exposes value types for:

- typed annotation keys and the bounded V1 scalar/array value set;
- UUID, session, surface, page, and element identity;
- structured page paths, relations, exposure, and focus;
- behavior kind, operation, payload, capture class, and redaction state;
- monotonic and wall-clock snapshots;
- deterministic sampling rates and decisions; and
- idempotent lifecycle state transitions.

Names, annotation values, arrays, page depths, and effective annotation counts
enforce the canonical V1 bounds. Invalid or unknown values fail closed.

### Outer-first context

`AnnotationContext` is immutable. Each added scope uses Swift collection
copy-on-write storage to create a new snapshot, preserving its parent and
siblings. Existing outer values are never overwritten. Every losing
declaration records its winning and losing origin and whether the values were
identical. A captured snapshot is already resolved; capture does not walk the
SwiftUI tree or merge dictionaries again.

### Runtime gate order

There is deliberately no public `track`, `record`, or `capture` method. The
runtime's recording seam has Swift `package` access, so only sibling SDK targets
can use it. Macro expansions compile in the adopting application's module; the
macro target will therefore add a narrow public generated-code support API that
delegates to this package seam rather than exposing a general event emitter.

The seam evaluates work in this order:

1. SDK enabled flag;
2. independent capture-class consent;
3. precomputed behavior or replay sampling decision;
4. lazy draft construction;
5. clock and UUIDv7 record identity;
6. a short unfair-lock sequence-number increment; and
7. synchronous submission to a non-I/O queue sink.

The first three gates return before evaluating the draft closure. Sink
implementations must enqueue only; disk, database, compression, and network
work belong to delivery workers.

`OSAllocatedUnfairLock` provides a small Apple-platform critical section while
retaining the deployment floor above. The newer standard-library `Mutex` would
raise that floor to iOS 18/macOS 15. No actor hop or task creation occurs on the
capture path.

### Sampling and time

The Swift sampler implements the cross-platform
`SHA-256("chill-sampling-v1\\0<stream>\\0<salt>\\0<stable-key>")` algorithm.
It interprets the first eight digest bytes as an unsigned big-endian integer.
Fractional thresholds use `dividingFullWidth`, avoiding floating-point drift
and preserving support below iOS 18/macOS 15. Behavior and replay decisions are
computed once per runtime and never inherit the OpenTelemetry trace sampled
bit.

Wall and monotonic time are captured together. Durations and replay alignment
use only monotonic time within one boot ID. Injected clocks and ID generators
make conformance tests deterministic without exposing controls in the
developer-facing API.

### Privacy

Essential, analytics, diagnostic, and replay consent are independent. Unknown
or denied states stop capture before draft construction. Secure inputs and the
clipboard remain masked even if mistakenly added to an explicit allowlist.
Replay text, form state, accessibility text, and pixels produce masks rather
than raw content by default. Network bodies, dynamic URLs, error descriptions,
and arbitrary payloads remain omitted unless a later project schema explicitly
classifies a safe bounded value.

### Performance evidence

`ChillCoreBenchmarks` measures both the disabled and enabled package seam in a
release build. The initial local smoke run of one million calls measured about
2.5 ns per disabled declaration and 79 ns per enabled in-memory submission,
with zero disabled draft executions. These numbers are a regression signal,
not a substitute for the physical-device release measurements required by
ADR-0006.

Run it with:

```sh
scripts/run-swift-core-benchmarks.sh
```

## Consequences

- Future macros and UI adapters can share one concurrency-safe semantic core.
- Application developers cannot discover or call a tracking API.
- Denied and disabled declarations have a directly testable no-work invariant.
- Export and replay stay out of the core binary until explicitly linked.
- A future non-Apple Swift server core will need a portable lock abstraction;
  that does not broaden the Apple vertical slice now.
