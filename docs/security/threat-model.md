# Chill repository threat model

## Overview

Chill is a privacy-sensitive observability and product-behavior platform. Its
primary runtime is a Rust modular monolith (`chilld`) backed by PostgreSQL and
S3-compatible object storage. Native and browser SDKs send authenticated OTLP
behavior, trace, metric, and structural-replay data. The project console lets
authorized users configure projects, environments, sources, schemas,
credentials, sampling, retention, and privacy policy. Background workers
normalize durable inbox records, publish immutable Parquet, execute bounded
DuckDB queries, and perform retention, export, and deletion workflows.

The most important assets are tenant isolation, raw and normalized telemetry,
replay ciphertext and keys, credentials and the external pepper, verified
identity bindings, privacy/consent policy, deletion/export correctness,
immutable lake manifests, audit history, release artifacts, and the ability to
keep ingest available without losing acknowledged data.

Primary runtime code lives in `backend/crates`, `backend/migrations`,
`sdk/swift`, the client-only `console`, and `deploy/single-node`. Contract,
policy, schema, and conformance files are security controls because every SDK
and server implementation consumes them. Examples, validation fixtures, and
developer scripts are not production request surfaces, but release and CI
scripts are privileged supply-chain surfaces.

## Threat Model, Trust Boundaries, and Assumptions

### Trust boundaries

1. **Untrusted client to edge/runtime.** OTLP headers, compression metadata,
   protobuf/JSON bodies, replay chunks, trace context, baggage, record IDs,
   timestamps, and retry patterns are attacker-controlled. TLS termination and
   request limits are assumed to precede or be enforced by `chilld`.
2. **Credential to tenant scope.** A typed `ch_sk_`, `ch_us_`, or `ch_sv_`
   credential is untrusted until its prefix, digest, expiry, status, resource
   hierarchy, and current role are verified. Tenant IDs in payloads are never
   authoritative.
3. **Browser console to Rust API.** Console source and runtime state are
   untrusted client inputs. The server authenticates every request and enforces
   capabilities. A configured cross-origin console origin is operator
   controlled; it is not a wildcard trust grant.
4. **Verified identity adapter to session issuer.** Only a trusted adapter may
   call the verified-identity session boundary after checking signature,
   issuer, audience, nonce, and freshness. Email or forwarded display headers
   alone are not verified identity.
5. **Runtime to PostgreSQL.** The runtime role is less privileged than the
   migration/admin role. Every tenant transaction installs one organization
   context; forced RLS and scoped foreign keys are defense in depth. Database
   superuser compromise is outside this application boundary and is an
   operator incident.
6. **Runtime to object store.** Bucket credentials, endpoint, region, and
   path-style configuration are operator controlled. Object keys and manifests
   derived from tenant data are validated. Production assumes authenticated
   TLS; plain HTTP is acceptable only on an explicitly isolated local network.
7. **Immutable objects to DuckDB.** Queries may read only authenticated,
   committed manifest objects through typed and parameterized plans. Object
   listings and arbitrary SQL are not discovery or query interfaces.
8. **SDK application to SDK runtime.** Application annotations, navigation,
   custom values, URLs, errors, and UI structure may contain sensitive data.
   Consent, classification, masking, allowlists, sampling, and resource budgets
   must apply before buffering or replay capture.
9. **Replay producer to replay consumer.** Replay content must be masked at
   source and encrypted before transport. The server may index trusted metadata
   but must not silently gain cleartext access. Corruption, gaps, and missing
   keys fail closed.
10. **Developer/CI to release.** Contributors control source changes but not
    protected release credentials. Actions and container bases are digest or
    commit pinned; dependencies are locked; release attestations bind artifacts
    to source. A compromised developer workstation remains a realistic path to
    malicious code and requires review/branch protection outside this repo.

### Security invariants

- An authenticated principal can affect or observe only its current tenant and
  capabilities; changing a membership or revoking a credential takes effect on
  the next request.
- Acknowledged telemetry has a durable tenant-stamped receipt, and retries
  cannot create a second logical record.
- Untrusted values never become SQL structure, filesystem traversal, object
  prefix authority, response headers, or tenant selectors.
- Compressed, decompressed, record-count, queue, query, replay, and concurrency
  work is bounded before expensive processing.
- Sensitive data is omitted or masked before queues, logs, crash artifacts,
  replay chunks, or analytics files; consent withdrawal narrows collection and
  purges governed state.
- Raw credentials and pepper material are never stored or logged. One-time
  credentials are revealed exactly once and rotation revokes predecessors.
- Published lake objects and manifests are immutable, content verified, and
  discoverable only after a committed manifest.
- Export, retention, and deletion are scoped, idempotent, audited, and cannot
  silently skip a region, object, cache, or durable queue.
- Client-side TypeScript, Swift, and Kotlin may implement SDK/UI behavior, but
  all Chill-controlled backend logic remains Rust.

### Assumptions and exclusions

PostgreSQL, the object store, TLS termination, DNS, the host kernel, Apple
platform security, and GitHub's protected secret store are trusted dependencies
whose compromise is an operational incident, though Chill must minimize the
resulting blast radius. Denial of service from traffic exceeding the designed
host is in scope; volumetric upstream saturation beyond the edge provider is
not solvable solely in this repository. Physical compromise of an unlocked
end-user device, malicious app code intentionally reading its own memory, and
an organization owner deliberately exporting data they are authorized to
export are outside the application authorization model.

## Attack Surface, Mitigations, and Attacker Stories

### Ingestion and protocol handling

An attacker with a stolen SDK key can spray malformed protobuf/JSON, oversized
gzip streams, extreme record counts, duplicate IDs, adversarial trace/baggage
headers, or slow requests. `chill-ingest` authenticates typed credentials,
caps compressed/decompressed size and expansion ratio, validates media type
and record counts, stamps tenant scope server-side, and acknowledges after a
PostgreSQL transaction. Tests cover idempotency, backpressure, header
ambiguity, and canonicalization. High-risk residuals are parser variance,
quota races, slowloris behavior at the proxy/runtime boundary, and starvation
between ingest classes; fuzz and abuse suites must retain corpus coverage.

### Authentication, authorization, and console

Attackers may steal, replay, rotate, or substitute credential types; exploit a
stale role; attempt cross-tenant IDs; abuse a permissive console origin; or use
XSS to read a one-time key. Credential prefixes are type-separated, secrets are
peppered and constant-time verified, roles resolve from current membership,
mutations require explicit capabilities, PostgreSQL RLS is forced, and console
responses are `no-store` with restrictive headers. The console keeps the active
session only in memory/session scope and never persists raw SDK credentials
after the one-time reveal. Production identity/session transport, CSRF policy,
organization switching, CSP for static console assets, and login abuse limits
remain activation requirements before external SaaS.

### Query and analytics

An authorized reader may supply property filters, time ranges, funnel steps,
cohort predicates, or IDs designed for injection, excessive scans, memory
exhaustion, cross-tenant manifest selection, or cache confusion. The Rust query
layer accepts typed plans, binds untrusted values, authenticates scope before
manifest selection, caps time/range/files/rows/memory/concurrency, and keys
cache entries by scope and plan. Arbitrary SQL, object globbing, and client
paths are prohibited. Product-facing query APIs must preserve these types and
limits rather than wrapping raw SQL.

### Object storage, replay, and lifecycle

Attackers may attempt traversal-like keys, manifest/object substitution,
replay flooding, ciphertext corruption, stale acknowledgements, content
leakage through metadata, or deletion/export incompleteness. Keys are derived
from validated identifiers and content digests; writes are immutable and
verified; manifests commit after objects; replay capture masks before
encryption; queues quarantine corruption and enforce caps; lifecycle operations
use leases, idempotency, audit, and verification. A custom `http://` S3 endpoint
is safe only for the isolated single-node network and must be rejected or
explicitly opted into for any external deployment.

### SDKs and macro/compiler tooling

Host application values, source annotations, UI text, accessibility labels,
form values, network destinations, errors, and macro arguments can leak data or
expand into unsafe code. Swift macros enforce literal semantic names and
preserve effects/isolation; SDK runtimes use typed annotation values,
outer-authoritative merge rules, allowlisted trace propagation, default-closed
consent, source masking, bounded queues, and golden conformance fixtures.
Release testing must include compiler-version drift, generated-code review,
privacy canaries, accessibility behavior, performance/resource limits, and
offline recovery on the oldest supported physical OS.

### Supply chain and operations

A dependency, action, container image, release script, migration, or stolen CI
credential can subvert every tenant. Cargo and Swift locks, pinned toolchains,
immutable released migration checksums, pinned Actions/images, reproducible
source archives, OCI attestations, Clippy/format/tests, and RustSec/source-policy
checks reduce this risk. Remaining controls include SBOM publication, periodic
dependency review, secret scanning, security static analysis, protected release
approval, and a signed release evidence bundle. The informational unmaintained
`paste` dependency is transitive through Apache Arrow Parquet and is tracked
until Arrow removes it; vulnerabilities are not allowlisted.

## Severity Calibration (Critical, High, Medium, Low)

### Critical

- Unauthenticated or ordinary-tenant remote code execution in `chilld`, CI, or
  release artifacts.
- A systematic authorization/RLS bypass exposing or mutating many tenants.
- Extraction of stored credentials, pepper, replay keys, or cleartext replay at
  platform scale.
- A release/supply-chain compromise that ships attacker code to SDK consumers
  or production while satisfying normal verification.

### High

- Reliable cross-tenant read/write for one neighboring tenant, credential
  issuance outside the caller's scope, or current-role bypass.
- A remotely reachable decompression/parser/query path that exhausts the
  standard deployment with modest traffic or loses acknowledged data.
- Source masking, consent, export, or deletion failure that exposes or retains
  sensitive user data beyond policy.
- Manifest/object substitution causing trusted analytics to read attacker-
  selected content.

### Medium

- Same-tenant privilege escalation requiring a developer/analyst account,
  bounded denial of service requiring sustained authenticated traffic, or
  limited metadata disclosure.
- Console XSS that requires an authorized victim but can steal a one-time key.
- Audit gaps that materially impede incident reconstruction without enabling
  the underlying unauthorized action.
- A non-default insecure operator configuration with clear warnings and no
  effect on standard deployment.

### Low

- Minor information exposure limited to non-secret versions or opaque IDs,
  low-amplification resource waste, or defense-in-depth header omissions with
  no practical exploit path.
- Developer/test tooling issues that require local write access and cannot
  influence release or production artifacts.
- Availability or correctness defects confined to synthetic examples without a
  path into runtime contracts.
