# ADR-0029: Portable Rust and desktop runtime SDK boundaries

- Status: Accepted
- Date: 2026-07-21
- Scope ticket: CHILL-97

## Context

Chill's Apple, Android, and browser SDKs implement the shared behavior contract,
but generic Rust applications, Linux processes, Tauri hosts, Electron main
processes, and JavaScript runtimes currently need to assemble raw OpenTelemetry
instrumentation themselves. Reusing the browser SDK in a webview covers DOM
behavior only. It does not cover process lifecycle, native commands, IPC,
crashes, updater state, or durable host-process delivery.

The expansion must preserve two existing boundaries:

- every Chill-controlled production service remains Rust under ADR-0026; and
- renderer and browser instrumentation must not gain access to native IPC
  payloads, credentials, filesystem content, or other host-only data.

## Decision

Chill adds four coordinated, independently releasable client components:

| Component | Location | Responsibility |
| --- | --- | --- |
| Portable Rust SDK | `sdk/rust` | Semantic records, async trace context, bounded durable delivery, lifecycle and panic hooks, trusted HTTP propagation |
| Runtime JavaScript SDK | `sdk/js/packages/runtime` | Node, Deno, and Bun host instrumentation over Web APIs and Node-compatible async/filesystem primitives |
| Electron adapter | `sdk/js/packages/electron` | Main-process lifecycle, windows, updater/crash facts, renderer setup, and metadata-only IPC correlation |
| Tauri adapter | `sdk/tauri` and `sdk/js/packages/tauri` | Rust-host lifecycle plus metadata-only command/event correlation with a browser-instrumented webview |

The portable Rust SDK is a standalone client workspace. It does not depend on
the `backend` workspace or import control-plane, ingest, storage, query, or
server configuration crates. Tauri depends on the public Rust client surface.
Electron and the JavaScript runtime packages never become production backend
implementations.

### Canonical source and resource identity

The existing V1 canonical `source.platform` remains a coarse contract family,
not a framework or operating-system field:

- Rust, Node, Deno, Bun, Electron-main, and Tauri-host records use `server`;
- Electron and Tauri webviews use `web` through the browser SDK; and
- `process.runtime.name`, `process.runtime.version`, `os.type`,
  `service.name`, and `chill.desktop.framework` resource attributes identify
  the actual runtime and framework.

This avoids inventing platform enum values that the V1 backend rejects and
keeps a correlated desktop session queryable across its host and renderer
records. The integration guides must always show both the canonical family and
the runtime/framework resource identity.

### Semantic authoring surface

There is still no generic `Tracker.track(name, properties)` API. Portable SDKs
offer bounded semantic operations:

- a declared instant event or action with a stable semantic name;
- an activity wrapper that owns start, terminal outcome, duration, and trace
  scope around one synchronous or asynchronous operation; and
- lifecycle adapters whose names and payload schemas are fixed by Chill.

Application values enter only through registered annotation keys with declared
classification and bounded primitive types. Function arguments, return values,
errors, IPC arguments, message bodies, headers, URLs, paths, environment
variables, and stack-local values are never captured automatically. Errors map
to a stable reason code; descriptions and backtraces are excluded.

Collection consent is checked before IDs, serialization, persistence, or
network work. Denial aborts active export where the runtime supports
`AbortSignal` or cancellation, purges the durable queue, and prevents further
capture. Credentials remain memory-only and are never written beside queued
records.

### Delivery and process lifecycle

The Rust and JavaScript runtime SDKs use a bounded at-least-once record store.
The store persists already-redacted records with their original UUIDv7 record
IDs, writes atomically, acknowledges only a successful OTLP response, and
retains records after retryable failure. Corrupt entries fail closed and are
reported through bounded diagnostics without logging their content.

High-value lifecycle, action, event, and activity records are retained before
low-value impressions or optional diagnostics. No SDK performs network or disk
I/O while holding application callbacks. Applications explicitly choose the
queue directory and must provide an installation identity or allow the SDK to
create a random UUIDv4 identity there.

Process start, ready, background/suspend where available, resume, graceful
shutdown, updater, window, and crash/panic facts use fixed payload keys. Panic
and uncaught-exception hooks chain to the application's previous hook. They
record only a stable event class and never the panic message, exception
message, stack, file path, or payload.

### Trace and HTTP propagation

Portable runtimes use structured async-local context (`tokio` task-local state
or `AsyncLocalStorage`) and W3C Trace Context. Activities inherit the active
context and create a child span identity without making session, page, or
activity subject IDs into parents.

HTTP propagation is exact-origin allowlisted. The adapter injects only a valid
`traceparent` and explicitly allowlisted bounded baggage immediately before a
request. It never forwards credentials, arbitrary annotations, URL data, or
received baggage. Redirect destinations are re-evaluated before forwarding.
Trace sampling and behavior retention remain independent.

### Desktop IPC correlation

Electron and Tauri use a dedicated metadata envelope containing only:

- a version marker;
- a valid `traceparent`;
- an optional bounded public baggage map; and
- a developer-declared, low-cardinality semantic command name.

The envelope is transported beside application arguments. Chill adapters do
not enumerate, clone, stringify, hash, log, store, or annotate those arguments.
The host validates the metadata, starts or joins one activity, runs the real
handler with the untouched arguments, and returns the untouched result or
error. Invalid or absent metadata starts a local trace and produces a bounded
diagnostic; it never rejects the application command.

Channel names are configuration, not telemetry. Applications map a concrete
Electron channel or Tauri command to a stable semantic name. Wildcard channel
instrumentation is not supported.

### Conformance and releases

`conformance/v1` remains byte-for-byte semantically immutable. Portable-only
IPC, attribute-bound, and runtime propagation fixtures live in a separately
versioned portable extension and are required in addition to the applicable V1
`server.core` or `client.core` profile. Reports bind the suite digest and name
their runtime, adapter, layer, SDK version, and target.

Rust is checked on supported Linux GNU targets and the host development target.
Runtime JavaScript is executed under pinned Node, Deno, and Bun versions.
Electron and Tauri logic is tested against deterministic adapter doubles, with
consumer examples proving the public integration shape. Source archives remain
coordinated by `VERSION`; publishing to crates.io or npm is a separate,
credentialed release operation and is not performed by pull-request CI.

## Supported targets and non-goals

The normative matrix is `docs/platform-support.md`. Initial support covers
Rust applications on tier-1 Linux GNU targets, Tauri 2 hosts, Electron main and
renderer processes, and current pinned Node, Deno, and Bun releases.

The following remain out of scope:

- screenshots, pixels, arbitrary webview content, or native accessibility text;
- implicit inspection of IPC, message, job, database, filesystem, or function
  payloads;
- automatic instrumentation of every GUI toolkit or dynamic IPC channel;
- a JavaScript implementation of Chill's production backend;
- using trace context as authorization, tenancy, consent, or identity; and
- claiming native crash dump capture or exactly-once delivery.

## Consequences

- Generic Linux and desktop applications gain the same privacy and behavior
  vocabulary without importing the server platform.
- Host and renderer telemetry remain separately attributable but trace-
  correlated.
- Applications must use the provided semantic handler wrappers at IPC
  registration boundaries; transparent monkey-patching would violate the
  payload privacy rule.
- A dedicated portable conformance extension adds release work, but avoids
  breaking existing V1 reports or weakening their exact digest contract.
