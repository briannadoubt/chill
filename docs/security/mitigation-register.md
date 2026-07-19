# Security mitigation register

This register maps CHILL-40 threats to owned controls and evidence. A row is
closed only when the linked automated or dated manual evidence exists; design
intent alone is not closure.

| ID | Threat/control | Current evidence | Owner | Gate | State |
| --- | --- | --- | --- | --- | --- |
| SEC-001 | Cross-tenant access and credential substitution | Forced-RLS migrations; typed credentials; `postgres_console`, ingest, query, lake, lifecycle integration suites | platform | every release | controlled |
| SEC-002 | OTLP parser, gzip, record-count, and retry abuse | bounded ingest validator; durable/idempotent PostgreSQL tests | platform | every release | controlled; fuzz expansion due |
| SEC-003 | Query injection and resource exhaustion | typed plans, bound values, manifest-scoped reads, execution caps, query tests | platform | every release | controlled; product API abuse suite due |
| SEC-004 | Replay/source privacy leakage | source masking, consent purge, corruption quarantine, privacy canaries, conformance fixtures | apple + platform | Apple release | controlled; physical corpus due |
| SEC-005 | Object-key/manifest substitution | validated content-addressed keys, immutable writes, digest verification, commit-after-object tests | platform | every release | controlled |
| SEC-006 | Insecure custom S3 transport | single-node isolates HTTP on an internal Docker network | platform | before external SaaS | open: require explicit unsafe-local opt-in or TLS for non-local endpoints |
| SEC-007 | Vulnerable or untrusted Rust dependencies | `cargo deny check advisories sources`; legacy vulnerable TLS feature removed | platform | every PR | controlled |
| SEC-008 | Unmaintained `paste` via Apache Arrow Parquet | RustSec `RUSTSEC-2024-0436`; transitive, informational, no safe upgrade | platform | monthly dependency review | accepted temporarily; remove when Arrow no longer depends on it |
| SEC-009 | Static-analysis coverage | deny-warnings Clippy across all Rust targets/features, forbidden unsafe, TypeScript compiler/ESLint, Python contract validation | platform | every PR | controlled for 0.1; evaluate hosted CodeQL when Rust support is production-ready |
| SEC-010 | Fuzz/property coverage | credential, OTLP/replay candidate, and typed-query libFuzzer targets; weekly pinned nightly exercise; common conformance corpus | platform + SDK owners | weekly and before 0.1 | controlled; expand corpora when new parsers ship |
| SEC-011 | Auth spraying, slowloris, quota races, replay floods | per-request limits, quotas, backpressure, edge body cap; weekly full PostgreSQL/object-store abuse integration suite | platform | weekly and before 0.1 | controlled for application boundaries; volumetric edge exercise remains an operator drill |
| SEC-012 | Browser sign-in, CSRF, and org switching | opaque current-role sessions; client-only console; explicit CORS origin | product + platform | before external identity | open: verified Rust identity adapter and CSRF-safe browser transport |
| SEC-013 | Release/supply-chain compromise | pinned actions/images/toolchains, locks, reproducible archives, OCI provenance | release owners | every release | partial: SBOM, secret scan, security signoff bundle due |
| SEC-014 | Deletion/export incompleteness | audited idempotent lifecycle and restore/deletion integration drills | privacy + platform | every release | controlled; regional drill required before multi-region |
| SEC-015 | Physical Apple privacy/performance regression | simulator/host tests and failed-closed budget evaluator | apple | Apple release | open: oldest-OS physical 40-measurement report must pass |
| SEC-016 | External support/billing/abuse privilege | ADR-0027 interfaces, grants, lifecycle, and triggers | platform + privacy | before external SaaS | design complete; implementation inactive |

## Operating rules

- Vulnerabilities with a safe upgrade are fixed; they are not ignored to make
  the gate green. Informational notices may be time-bounded only with a named
  transitive path, rationale, and review gate.
- Any new public parser, credential type, query plan, object namespace, replay
  representation, or privileged workflow adds a row before release.
- `open` items block the gate named in their row, not unrelated internal
  development. CHILL-58 cannot close while any `before 0.1` or release-gated
  row remains open.
- Manual penetration, physical-device, and disaster-recovery evidence is dated
  and linked from Scope; it is never represented by an empty test harness.
