# ADR-0009: Swift activities and events use attached body macros

- Status: Accepted
- Date: 2026-07-14
- Owners: Apple SDK architecture
- Decision scope: Swift function instrumentation

## Context

Chill application code must describe behavior without calling an imperative
tracking API. Swift functions already define meaningful domain and operation
boundaries, but wrappers, duplicate overloads, and manual `defer` blocks alter
signatures or make lifecycle handling easy to get wrong. Instrumentation also
has to preserve actor isolation, generics, typed throws, cancellation, and
recursive or retried execution.

## Decision

The `Chill` Swift product exposes two attached body macros:

```swift
@Activity("repository.load", kind: .storage, role: .operation)
@Event("cat.loaded", emission: .succeeded)
```

Each macro replaces only the attached function's body. The compiler retains
the original declaration, including its access level, parameters, ownership,
generic constraints, result type, global actor, `async` effect, and typed error
type. The generated body invokes an underscored support wrapper around the
original statements. No shadow overload or detached task is created.

Swift currently erases a typed error when a throwing operation crosses the
standard library's `TaskLocal.withValue` rethrows boundary. For a typed-throws
function, the macro therefore gives the generated closure its original typed
error context. The runtime converts the captured result back to that exact
failure type after leaving the task-local scope. Expansion and compiled
integration tests make this a release invariant.

### Names and diagnostics

Activity and event names must be static, non-interpolated string literals that
meet the canonical 128-byte semantic-name grammar. The macro validates them at
compile time. Option values remain ordinary type-checked Swift expressions.

Function events support `entered`, `succeeded`, and `terminal` emission.
`observed` is rejected at compile time because it describes a declarative state
transition and belongs to the UI modifier surface. Macros reject unsupported
declarations instead of emitting a partially instrumented body.

### Activity lifecycle

An enabled activity emits a start record before its body and exactly one end
record after success, failure, or cancellation. A task-local frame carries
parent identity through structured concurrency without a thread-local or actor
hop. Recursive calls receive increasing recursion depth and distinct activity
identities.

An activity with role `attempt` records only below an operation activity. Its
parent frame assigns one-based attempt numbers for each semantic attempt name.
This makes retries explicit without guessing from repeated network requests.

### Event lifecycle

An entered event emits before the body. A succeeded event emits only after a
successful result. A terminal event emits after success, failure, or
cancellation with the corresponding outcome. Optional deduplication keys are
SHA-256 hashed before buffering; the raw key is never placed in a record.
Deduplication is scoped to the configured runtime and semantic event name.

The wrappers never capture arguments, return values, error descriptions, or
arbitrary payloads. Those values require an explicit, bounded, privacy-typed
annotation surface.

### Disabled path and dependencies

The process-wide configured-runtime check is backed by a C11 atomic Boolean.
Disabled declarations execute their original body without acquiring the Swift
runtime lock, building a draft, reading a clock, generating an ID, or touching
a sink. Configuration changes store or clear the runtime behind a short unfair
lock and publish the enabled state with acquire-release ordering.

SwiftSyntax is a host-side compiler-plugin dependency pinned to the matching
Swift 6.4 release. It is not linked into an application. The runtime adds only
the tiny in-package C atomics target; no third-party runtime SDK is introduced.

## Consequences

- Application functions contain no telemetry calls or lifecycle bookkeeping.
- Macro expansion is deterministic and snapshot-testable.
- Typed errors, actor isolation, cancellation, recursion, and attempts have
  compiled behavioral coverage.
- The compiler permits only one attached body macro on a function. A function
  therefore declares either its activity boundary or one event boundary; UI
  state events remain separate declarative modifiers.
- The underscored generated-code support functions must remain public for code
  expanded into client modules, but they are hidden from documentation and are
  not a general record-building API.
