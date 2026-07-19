# ADR-0004: Declarative actions, activities, and events

- Status: Accepted
- Date: 2026-07-14
- Scope ticket: CHILL-6
- Reference implementation: `contracts/instrumentation/v1/reference.py`

## Context

Chill must capture important UI and domain behavior without asking application
code to call `Tracker.track()`, manually create spans, or remember success and
failure callbacks. The system needs three different semantic shapes:

- a user or platform command committed through a native UI element;
- time-bearing work that starts and reaches one terminal outcome; and
- an instantaneous domain or lifecycle fact.

These shapes need exact behavior across repeated SwiftUI rendering, async
suspension, errors, cancellation, recursion, retries, offline duplicate
delivery, and repeated processing of an idempotent business message. Arbitrary
function arguments, return values, error descriptions, button labels, and
entered text cannot be collected implicitly.

Swift is the first complete SDK and Swift body macros are a product requirement.
Swift Evolution SE-0415 was implemented in Swift 6.0 specifically to let an
attached macro replace or augment a declared function body, including tracing
wrappers. A function accepts at most one body macro, so Chill must provide one
complete transformation per instrumented declaration instead of relying on
macro-order composition.

## Decision

Chill exposes three declarative concepts with separate identity and lifecycle
rules:

| Concept | Declaration | Observation point | Canonical shape |
| --- | --- | --- | --- |
| Action | `.action("cat.adopt")` on a semantic native control | The platform commits one activation | One instant `action` fact |
| Activity | `@Activity("adoption.submit")` on a function | Entry and terminal exit of every invocation | `activity.start` plus at most one `activity.end` |
| Event | `@Event("adoption.confirmed")` on a function | Declared entry/success/terminal point | Zero or one instant `event` fact per invocation |

These are the only public authoring mechanisms in V1. There is no public
`track`, `startActivity`, `finishActivity`, or `emitEvent` API. Generated macro
code and native adapters call an internal runtime that application code cannot
import directly.

### Semantic names

Names are stable, low-cardinality, developer-controlled identifiers. They do
not come from localized labels, Swift symbols, type descriptions, URLs, or
argument values. A rename is a schema change visible in analytics.

The application may use generated constants from its project schema catalog,
but string literals remain valid for the initial internal release:

```swift
Button("Adopt", action: submit)
    .action("cat.adopt")
    .annotation("cat.id", value: cat.id)
```

The button label remains accessibility and presentation content. It is never
silently promoted to `name` or captured as an annotation.

## Declarative actions

`.action` binds metadata to the nearest eligible native semantic control. The
declaration itself is inert: creating, diffing, mounting, laying out, reusing,
or rendering a declared view emits no action.

An action is emitted after the platform recognizes a committed command, not on
raw touch-down, pointer-down, key-down, hover, focus, or a gesture that is later
cancelled. Disabled controls do not emit. Keyboard, remote, voice, and
assistive-technology activation are first-class input origins and must not be
misreported as pointer or touch input.

The normalized payload separates semantic activation from input origin:

- `activation`: `primary`, `submit`, `toggle`, `selection`, `adjust`,
  `gesture`, or `system`;
- `input`: `touch`, `pointer`, `keyboard`, `remote`, `accessibility`, `voice`,
  `system`, or `unknown`; and
- `role`: the platform-normalized semantic role such as `button`, `link`,
  `menu_item`, `switch`, `picker`, or `form`.

Arbitrary programmatic function calls are domain work, not user actions. They
become actions only when a supported platform command system reports an actual
semantic activation; otherwise an `@Activity` or `@Event` declaration expresses
their meaning.

### Normalization and duplicate-render protection

Every platform adapter creates one short-lived `native_activation_id` at the
earliest trustworthy semantic command boundary. Observer layers may then see
the same command through accessibility, gesture, control dispatch, focus,
navigation, and framework callbacks. They all carry the same ID.

The runtime emits at most one action for `(surface_id, native_activation_id)`
and keeps a bounded recent-ID set. The ID is an SDK-local deduplication token;
the exported action's `record_id` remains its durable identity. Two actual user
activations receive different native IDs even when they occur in the same frame.

SwiftUI value reconstruction cannot create a new semantic element lifetime by
itself. The adapter keys a declaration by its stable semantic element instance,
modifier slot, and native target—not by a transient `View` value, an array
index, object address, or UUID created during `body` evaluation.

If several nested declarations appear eligible for one native command, the
adapter selects the closest declared semantic command target. Raw trigger
events that lead to a higher-level committed command, such as a pointer click
that commits form submission, do not become a second action. Ambiguous layouts
produce a development diagnostic rather than duplicate analytics.

At activation, annotations are resolved outer-first and snapshotted. The
runtime establishes an ephemeral activation context while the native callback
is dispatched so generated activities started by that callback correlate with
the action automatically.

## Swift activity macro

`@Activity` is an `@attached(body)` macro on a concrete function or method with
a body. It supports synchronous, asynchronous, throwing, nonthrowing, generic,
actor-isolated, and overloaded functions.

```swift
@Activity(
    "adoption.submit",
    kind: .domain,
    capture: [
        .argument("catID", as: "cat.id", classification: .identifier),
    ]
)
func submit(catID: Cat.ID) async throws -> Receipt {
    try await service.adopt(catID)
}
```

Conceptually, the macro re-emits the original body inside the matching internal
sync/async and throwing/nonthrowing runtime wrapper:

```swift
func submit(catID: Cat.ID) async throws -> Receipt {
    try await _ChillMacroRuntime.withActivity(
        descriptor: /* generated static descriptor */,
        captures: /* generated, evaluated once */
    ) {
        try await service.adopt(catID)
    }
}
```

The generated call is implementation detail, not developer-authored telemetry.
The expansion executes the original body on the same actor and executor. It
does not introduce an unstructured task or actor hop, change the function's
signature, erase generic constraints, swallow or wrap errors, alter the return
value, or change cancellation behavior. Its body closure is nonescaping.

When collection is disabled, the wrapper's first branch invokes the body
directly without generating IDs, snapshots, or export work. Export enqueueing
never blocks the calling actor.

### Activity lifecycle

Each function invocation creates a new UUIDv7 activity subject and follows this
state machine:

```text
new -> active -> succeeded
              -> failed
              -> cancelled
              -> timed_out
```

Entry emits one `activity.start`. A normal return emits one `activity.end` with
`outcome.status = ok`, including `Void` returns. A thrown error emits one end
with `error` and the exact original error is rethrown. A recognized cancellation
error emits `cancelled`; a recognized timeout emits `timeout`.

Checking `Task.isCancelled` is not enough to rewrite a normal return. If the
function observes cancellation and deliberately returns a value, its outcome is
`ok`. This keeps the macro from guessing application semantics. A fatal trap,
process kill, crash, or power loss can leave a valid start without an end.

Terminalization is idempotent. Racing cleanup paths, exporter recovery, or a
duplicate callback cannot emit a second end for the same subject. The canonical
record itself remains at-least-once deliverable: a retried envelope keeps the
same `record_id`, and ingest deduplicates that record ID.

Duration uses the source monotonic clock. Error type names, associated values,
localized descriptions, stack traces, and response bodies are not annotations
or reason codes. A project may explicitly map a typed error case to a stable,
low-cardinality reason code; all other errors use a bounded generic code.

### Async context and child work

The runtime stores the current activity and trace context in Swift task-local
state. It survives `await` and propagates through structured child tasks. A
child activity records the Chill parent activity ID independently of its OTel
span parent.

An explicitly detached task is a new context boundary and does not inherit a
parent implicitly. Future APIs may support a typed, policy-checked context
token for deliberate detached or callback bridging; V1 never asks developers
to copy raw trace IDs.

For action and event facts, `context.activity_id` identifies the nearest active
activity when the snapshot is taken. For an activity fact, the same context
field and `payload.parent_activity_id` both identify its immediate Chill parent
when one exists. The current activity remains the record's own `subject_id`, so
parent and child identity cannot be confused.

A child created while a parent context is active may finish after the parent's
end. Its recorded parent remains the captured parent subject. Starting unrelated
work from a stale or forged ID is not permitted by a native SDK.

### Recursion

Every recursive invocation is a new child activity. `recursion_depth` is the
count of ancestors with the same activity definition: the outer call is zero,
its recursive child is one, and so on. Recursion is never coalesced because each
invocation can have its own duration and outcome.

Sampling or depth budgets may suppress descendants after a configured limit,
but suppression must preserve the program and emit a throttled SDK diagnostic.
It cannot turn several invocations into a misleading single duration.

### Retries

Retries cannot be inferred reliably from identical consecutive calls. V1 uses
an explicit declarative coordinator/attempt shape:

```swift
@Activity("cat.load", role: .operation)
func loadCat(id: Cat.ID) async throws -> Cat {
    try await retryPolicy.run {
        try await loadCatAttempt(id: id)
    }
}

@Activity("cat.load.attempt", role: .attempt)
private func loadCatAttempt(id: Cat.ID) async throws -> Cat {
    try await service.loadCat(id: id)
}
```

The retry library is ordinary business control flow; neither closure contains
a telemetry call. An attempt must have a current parent activity. Attempts with
the same activity definition under that parent receive ordinals 1, 2, 3, and
so on in invocation order. Each attempt has its own subject and outcome. The
coordinator describes the logical operation and ends with the final result.

Parallel attempts are allowed. Their ordinal is allocation order, not completion
order. An `.attempt` declaration without a parent is a compile-time diagnostic
when statically knowable and a suppressed runtime diagnostic otherwise.

## Swift event macro

`@Event` is also an `@attached(body)` macro. It wraps one function invocation
and emits according to an explicit policy:

| Policy | Emission point | Behavior on thrown error/cancellation |
| --- | --- | --- |
| `.succeeded` | Immediately after normal return; default | No event |
| `.entered` | Before the original body | Already emitted, with no guessed outcome |
| `.terminal` | After any terminal exit | Emits actual ok/error/cancelled/timeout outcome |

```swift
@Event(
    "adoption.confirmed",
    capture: [
        .argument("catID", as: "cat.id", classification: .identifier),
    ]
)
func adoptionConfirmed(catID: Cat.ID) {
    state.markAdopted(catID)
}

@Event("sync.finished", emission: .terminal)
func synchronize() async throws {
    try await worker.synchronize()
}
```

Every invocation reserves at most one event record. Recursive invocations are
independent. Repeated terminal cleanup emits nothing after the first terminal
transition. The default `.succeeded` policy makes a function named for a state
transition truthful: a failed body does not claim the transition occurred.

The canonical event payload also permits `emission = observed` for automatic
SDK and server facts such as lifecycle, crash, and performance observations.
That value is not an `@Event` policy because there is no wrapped function
invocation whose entry or exit defines it.

Because Swift permits at most one body macro on a function, `@Activity` and
`@Event` cannot appear on the same declaration and cannot be stacked with a
third-party body macro. Chill emits a targeted compiler diagnostic explaining
the conflict. Use the activity's terminal outcome, or call a separately
annotated domain function, rather than depending on macro expansion order.

## Explicit capture without tracking calls

Macros capture no arguments, `self` properties, results, or errors by default.
Capture descriptors are compile-time declarations consumed by the macro:

- `.argument("parameter", as: "annotation.key", classification: ...)` reads a
  named parameter once at entry;
- `.result(as: "annotation.key", classification: ...)` reads a supported scalar
  result only after success; and
- typed error-to-reason mappings produce low-cardinality reason codes, never
  arbitrary error strings.

The macro verifies that a named parameter exists and generates ordinary Swift
expressions whose types are checked by the compiler. Unsupported, mutable,
nested, non-Sendable, or unclassified values produce compile diagnostics. A
capture expression is evaluated once in declaration order. It cannot alter
argument evaluation or cause a result to be recomputed.

Captured values become an inner annotation scope. The normal outer-first rule
still applies: a root declaration for the same key wins and the macro capture
produces a collision diagnostic. Activity entry captures one immutable context
snapshot and uses it through its terminal record. Events snapshot at their
declared emission point. Actions snapshot at committed activation.

### Semantic event idempotency

Normal event invocations are distinct even when their values are equal. For
idempotent message processing or repeated delivery of the same business fact,
an event can declare a stable opaque argument as an idempotency source:

```swift
@Event(
    "message.received",
    idempotency: .argument("messageID", classification: .opaqueIdentifier)
)
func apply(messageID: Message.ID, payload: Message) throws {
    // business logic only
}
```

The raw value is not exported. The SDK canonicalizes the supported scalar and
computes a domain-separated SHA-256 digest scoped by project and event name.
The digest is still classified as a pseudonymous identifier and can only be
enabled for an allowlisted opaque, sufficiently random source. Email addresses,
user-entered values, and other enumerable identifiers are forbidden.

The runtime suppresses duplicates it has already observed locally. Ingest also
enforces first-accepted semantics for `(tenant, event name, digest)` over the
configured idempotency retention window. The first event remains canonical;
later semantic duplicates become bounded diagnostics. This is separate from
transport idempotency, where resending the same envelope uses the same
`record_id`.

## Cross-platform contract

Swift macros are the first authoring experience, not a different data model.
Equivalent platform implementations use the most idiomatic compile-time tool:

- Kotlin annotations/compiler tooling wrap declared functions with the same
  invocation lifecycle;
- TypeScript decorators or build transforms wrap methods where supported;
- languages without a safe transform may offer generated wrappers, but may not
  expose an imperative tracking primitive as the primary workflow; and
- native UI adapters normalize semantic commands from their platform event and
  accessibility systems before action deduplication.

All platforms emit the same activity attempt/recursion fields, event emission
policy, action activation/input split, outcomes, snapshots, and diagnostics.

## Compiler diagnostics

The Swift macro implementation must fail compilation for contract violations
that syntax can prove:

| Code | Condition |
| --- | --- |
| `macro.invalid_name` | A name or annotation key violates the grammar |
| `macro.missing_body` | The declaration has no concrete body |
| `macro.unsupported_declaration` | Applied to a closure, property, accessor, deinitializer, or unsupported declaration in V1 |
| `macro.body_conflict` | Another body macro is attached |
| `macro.capture_missing` | A capture names no parameter/result |
| `macro.capture_unsupported` | The captured type/value shape is unsupported |
| `macro.capture_unclassified` | A sensitive capture lacks required classification |
| `macro.idempotency_unsafe` | An event key is not an allowlisted opaque identifier |
| `macro.attempt_context` | An attempt is statically outside an activity coordinator |

Diagnostics include a fix-it where the repair is mechanical. Runtime
diagnostics are throttled and never contain captured values.

## Required invariants

Every implementation must satisfy:

1. **No render emission:** declarations and UI reconstruction emit no actions.
2. **One semantic command:** one normalized native activation emits at most one
   action despite multiple observer callbacks.
3. **Distinct actual activations:** separate committed commands remain separate.
4. **One activity start:** every enabled macro invocation emits one start.
5. **At-most-one terminal:** an activity subject emits no more than one end.
6. **Signature transparency:** generated instrumentation preserves function
   result, error identity, cancellation, isolation, generic behavior, and
   evaluation count.
7. **Cancellation truth:** only a cancellation exit is cancelled; a normal
   return is successful even if the task's cancellation flag was set.
8. **Recursive identity:** each recursive invocation has a fresh subject and
   correct ancestry/depth.
9. **Retry truth:** attempts are numbered only under an explicit coordinator;
   identical calls are not guessed to be retries.
10. **Event cardinality:** an event invocation emits zero or one record at its
    declared emission point.
11. **Default-deny capture:** no argument, result, error string, UI label, or
    entered text is captured without an explicit classified declaration.
12. **Outer-first snapshot:** macro captures cannot overwrite established outer
    annotations.
13. **Transport idempotency:** retries of one envelope retain its record ID.
14. **Semantic idempotency:** an opted-in event key suppresses later accepted
    duplicates without exposing the raw key.
15. **Disabled fast path:** disabled instrumentation does not allocate IDs,
    snapshots, or export records and never changes application behavior.

The executable model and conformance tests live under
`contracts/instrumentation/v1` and `tests/conformance`.

## Consequences

- Important UI and domain behavior is readable at its declaration site without
  telemetry plumbing in the function body.
- Swift macros become a required, snapshot-tested part of the Swift SDK rather
  than optional sugar.
- One-body-macro limitations are explicit and produce good diagnostics.
- Retry and idempotency semantics require deliberate declarations because
  guessing would corrupt analytics.
- Privacy is safer by construction: names provide meaning, while runtime values
  remain excluded unless separately declared and classified.

## Primary references

- Swift Evolution, [SE-0415: Function Body Macros](https://github.com/swiftlang/swift-evolution/blob/main/proposals/0415-function-body-macros.md)
- The Swift Programming Language, [Macros](https://docs.swift.org/swift-book/documentation/the-swift-programming-language/macros/)
- The Swift Programming Language, [Concurrency](https://docs.swift.org/swift-book/documentation/the-swift-programming-language/concurrency/)
- Apple, [SwiftUI Button](https://developer.apple.com/documentation/swiftui/button)
- Apple, [SwiftUI accessibilityAction](https://developer.apple.com/documentation/swiftui/view/accessibilityaction%28_%3A_%3A%29)
