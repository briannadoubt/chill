# ADR-0012: Apple networking trace propagation

- Status: Accepted
- Date: 2026-07-15
- Scope ticket: CHILL-15
- Builds on: ADR-0005

## Context

Swift activities and native UI actions need to establish ordinary distributed
trace context without asking application developers to copy identifiers or call
a tracking API. URLSession must continue that context into first-party
services, but blindly attaching headers to every destination or redirect would
leak correlation and baggage to third parties. A mandatory OpenTelemetry SDK
dependency would also enlarge the smallest Chill installation and could install
a provider that conflicts with one already owned by the host application.

Network instrumentation must not inspect payloads. URL paths, queries,
authorization values, request and response bodies, and error descriptions can
all contain sensitive or unbounded data. The useful protocol facts are the
bounded method, origin components, response status, duration, redirect count,
and a normalized error class.

## Decision

### One task-local context and one provider

`ChillCore` owns an immutable, validated trace context in Swift task-local
state. Declarative native actions create a short `INTERNAL` dispatch span;
`@Activity` creates one `INTERNAL` activity span; and structured child tasks
inherit either context automatically. Canonical behavior records snapshot the
active trace and span identifiers independently of behavior sampling.

The ordinary public API exposes trace correlation as read-only data but offers
no initializer for raw identifiers. An underscored Swift SPI lets an official
adapter join the OpenTelemetry provider already installed by the application.
The bridge starts and ends spans and returns validated context; Chill never
sets or replaces a process-global provider. Without a bridge, Chill generates a
valid lightweight context for first-party propagation without exporting a
parallel span stream.

### URLSession adoption surface

`ChillNetworking` is a separate library product. An application opts an
existing session in once:

```swift
let propagation = try ChillNetworkPropagationPolicy(
  trustedOriginURLs: [URL(string: "https://api.example.com")!]
)
let network = URLSession.shared.chill(propagation)

let (data, response) = try await network.data(for: request)
```

This is networking configuration, not event emission. Application code never
calls `track`, supplies a trace ID, or annotates individual requests. The V1
facade supports modern async data, in-memory upload, and download entry points.
Streaming bytes remain outside V1 because ending a span when the byte sequence
is created would measure response headers rather than consumption; a future
stream wrapper must own the full sequence lifetime.

The facade uses a task-specific URLSession delegate. It does not swizzle
Foundation, register a global URL protocol, replace an application delegate, or
scan requests from unrelated sessions. The delegate re-prepares every redirect
request before Foundation follows it.

### Exact-origin propagation

The policy canonicalizes only HTTP and HTTPS origins. Scheme and host are
lowercase, default ports are removed, and origin configuration rejects
credentials, paths, queries, and fragments. Trust is exact: trusting
`https://api.example.com` does not trust a subdomain, another port, or an HTTP
endpoint.

When a propagation policy or trace bridge is active, Chill removes any existing
`traceparent`, `tracestate`, and `baggage` fields immediately before the first
send and every redirect. It restores them only when the destination's canonical
origin is explicitly trusted. This prevents a first-party header from riding a
redirect to a third party and prevents an untrusted request passed to this
safety boundary from carrying app-supplied correlation by accident. With no
trusted origins and no bridge, the facade takes a true no-op fast path and does
not install a delegate or mutate the request.

Trace Context parsing accepts the strict version-00 shape from ADR-0005:
lowercase nonzero 128-bit trace ID, lowercase nonzero 64-bit parent ID, and
lowercase one-byte flags. Tracestate is ASCII, unique-keyed, bounded to 32
members and 512 bytes, and rejects control characters, extra equals signs, and
trailing spaces. Invalid inbound context is discarded as a whole.

### Baggage and captured attributes

Baggage starts empty. A value is propagated only when its key appears in an
explicit allowlist. Keys use the W3C token grammar; output is sorted; and V1
enforces eight entries, 128 UTF-8 bytes per raw value, and 1,024 encoded header
bytes. Annotations and tenant, actor, session, page, action, activity, and replay
identities never enter baggage automatically.

The HTTP client span records only:

- `http.request.method`;
- `url.scheme`, `server.address`, and a non-default `server.port`;
- `http.response.status_code` and a normalized `error.type`;
- `http.redirect.count`; and
- a monotonic duration.

It never records a path, query, fragment, headers, cookies, credentials,
payload bytes, response bytes, or an error's localized description. HTTP error
status codes and transport failures end with `ERROR`; cancellation remains
`UNSET`.

## Consequences

- Native actions, macro activities, URLSession client work, and an instrumented
  backend can share one W3C trace without developer-managed identifiers.
- First-party trust is explicit and safe across redirects and subdomains.
- The default package remains small and provider-neutral; an OpenTelemetry
  adapter is additive.
- Applications adopt a narrow URLSession facade rather than receiving unsafe
  process-wide interception.
- Completion-handler and streaming entry points require later lifetime-correct
  adapters; V1 does not pretend incomplete timing is a complete client span.
