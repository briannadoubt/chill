# ADR-0027: External SaaS extends the internal control plane through explicit boundaries

- Status: Accepted
- Date: 2026-07-17
- Applies when: the migration triggers below are approved

## Context

Chill's internal deployment deliberately starts with one pre-provisioned
organization, a small role model, static quotas, and operator-run bootstrap.
Those constraints keep the first production system understandable to one
developer. They are not a safe external SaaS lifecycle. Public signup,
regional placement, billing, support access, abuse response, and tenant
deletion introduce privileged state machines that must not be improvised in
the project console or inferred from payment-provider webhooks.

This ADR defines the interfaces and activation triggers without adding unused
infrastructure to the internal MVP. ADR-0023 remains authoritative for typed
credentials, current-role authorization, and PostgreSQL tenant isolation.
ADR-0026 requires all SaaS control-plane services and workers to remain Rust.

## Decision

External SaaS is an additive Rust control-plane mode. The existing
organization, project, environment, credential, policy, and audit records
remain the tenant data plane. New SaaS records own public lifecycle decisions:

- `tenant_accounts`: plan, lifecycle state, home region, residency policy,
  suspension reason, and monotonic revision;
- `subscriptions`: provider-neutral customer/subscription references and the
  effective entitlement version, never card or bank data;
- `usage_ledger`: immutable, idempotent usage entries by tenant, environment,
  meter, interval, quantity, source receipt, and correction lineage;
- `support_grants`: requested scope, approver, reason, incident, expiry, and
  revocation for time-bounded support access;
- `abuse_decisions`: evidence references, policy rule, decision, reviewer,
  appeal state, and effective interval; and
- `regional_movements`: source and destination regions, dual-write/backfill
  phase, validation receipt, cutover, rollback deadline, and completion.

Provider payloads and secrets are not the source of truth. A verified webhook
becomes an idempotent command; the resulting local state transition and audit
record are authoritative. Raw payment instruments never enter Chill.

## Public interface contract

The external adapter exposes versioned commands whose bodies carry an
idempotency key and whose writes use optimistic `expected_revision` checks.
All successful mutations return the new revision and an audit ID.

| Interface | Authority | Result |
| --- | --- | --- |
| `POST /v1/signup/intents` | anonymous, rate-limited, verified email challenge | short-lived signup intent; no tenant yet |
| `POST /v1/signup/intents/{id}/complete` | verified identity + accepted terms | organization, owner membership, tenant account, and recovery-safe session in one transaction |
| `GET /v1/organizations` | verified identity | active memberships and allowed organization switches |
| `POST /v1/organizations/{id}/sessions` | verified identity + active membership | one organization-bound `ch_us_` session |
| `POST /v1/organizations/{id}/projects` | `control:write` | project with explicit region inherited from tenant placement |
| `POST /v1/projects/{id}/environments` | `control:write` | environment and initial quota/policy revisions |
| `GET /v1/usage` | `control:read` | finalized and provisional meter intervals with freshness |
| `POST /v1/billing/webhooks/{provider}` | verified provider signature | idempotent entitlement transition command |
| `POST /v1/support-grants` | tenant owner approval or audited break-glass authority | expiring least-privilege grant; no impersonation token |
| `POST /v1/tenant-lifecycle/{action}` | state-specific authority | suspend, reactivate, close, or erase transition receipt |

Browser identity assertions are verified by a dedicated Rust adapter for
signature, issuer, audience, nonce, and freshness before calling the existing
verified-identity session boundary. Email headers, client-supplied user IDs,
and billing-provider customer IDs can never issue a Chill session directly.
State-changing browser requests use same-site secure sessions or an explicit
CSRF token; console bearer tokens are not stored durably in browser storage.

## Regional placement and residency

Each tenant has one home region and an explicit residency policy. The global
directory contains only routing metadata, identity-to-membership lookup
material, and non-content lifecycle state. Telemetry, replay ciphertext,
query caches, exports, and deletion work remain in the tenant's allowed
region. Requests resolve tenant placement before accepting bodies and reject
region hints supplied by clients.

Movement is a reviewed state machine: `planned`, `dual_write`, `backfill`,
`verified`, `cutover`, `rollback_window`, `complete`. Every phase is
idempotent, has a checkpoint, and preserves deletion/retention holds. Cutover
requires row, object, digest, policy, and audit reconciliation. Rollback never
merges divergent writes silently. Cross-region analytics is not enabled by
default.

## Plans, quotas, metering, and billing

Entitlements compile into versioned limits consumed by existing admission and
query controls. A plan change never edits operational counters directly.
Quota reductions enter a pending state when current usage exceeds the new
limit; they cannot cause deletion or partial writes. Suspension behavior is
separate from quota exhaustion.

Meters are few, stable, and attributable: accepted behavior records, accepted
replay bytes, retained object bytes, query bytes scanned, export bytes, and
optional active environments. Each ledger insert has a unique source receipt
and interval. Corrections append inverse/replacement entries. A billing
adapter reads finalized intervals, submits provider-neutral invoice items,
and reconciles provider acknowledgement; webhook order never determines
usage. Cost and product analytics may consume the ledger but cannot mutate it.

## Support and abuse controls

Support access is an expiring grant, not account impersonation. The grant
names tenant, capabilities, resource scope, reason, incident, requester,
approver, expiry, and whether content access is permitted. Default grants are
metadata-only. Content, replay, export, credential, and deletion privileges
require separate explicit scopes. Every support request and read is audited
under the operator identity and is visible to tenant owners.

Break-glass requires a declared incident, two-person approval when staffing
permits, a short hard expiry, notification, and post-incident review. It cannot
disable tenant RLS, reveal stored credentials, or bypass source masking.

Abuse enforcement separates evidence collection from action. Decisions are
revisioned and appealable. Rate limiting protects anonymous identity and
signup routes; authenticated ingestion retains per-environment quotas and
backpressure. Automated signals may throttle, but destructive actions require
review unless an immediate legal or platform-safety obligation applies.

## Tenant lifecycle

The allowed lifecycle is:

`pending` → `active` ↔ `restricted` → `suspended` → `closing` → `erasing` → `erased`

- `restricted`: console and exports remain available; selected ingest/query
  operations are throttled or blocked by a recorded abuse/quota rule.
- `suspended`: ingest and ordinary queries stop; owners may sign in, resolve
  billing/abuse state, export when legally allowed, and request reactivation.
- `closing`: new writes stop and the contractual recovery window runs.
- `erasing`: credentials are revoked, workers execute the existing audited
  deletion protocol in every region, and reactivation is impossible.
- `erased`: only minimum legally required billing, abuse, and deletion receipts
  remain under a separate retention policy; tenant content is unavailable.

Legal hold is an explicit scoped record and never an undocumented deletion
failure. Provider cancellation requests `closing`; it does not erase data
directly. Reactivation is idempotent and allowed only before `erasing`.

## Migration triggers

None of this machinery is activated merely because code exists. The internal
mode remains authoritative until an owner approves all relevant triggers:

1. **Public identity:** the first non-employee tenant requires the verified
   identity adapter, MFA/recovery policy, CSRF-safe browser session transport,
   login abuse tests, and organization-switch audit coverage.
2. **Paid plan:** the first invoice requires the immutable usage ledger,
   meter reconciliation, entitlement revisions, refund/correction handling,
   and finance-approved provider boundary.
3. **Multiple regions or residency promise:** any contractual region choice
   requires the directory, movement state machine, regional deletion/export
   drills, and fail-closed routing before signup advertises that choice.
4. **Operator content access:** any support workflow that can inspect tenant
   content requires support grants, expiry enforcement, owner-visible audit,
   break-glass review, and the CHILL-40 security signoff.
5. **Automated suspension:** any automated enforcement beyond bounded rate
   limiting requires the abuse-decision record, appeal path, recovery tests,
   and documented behavior for ingest, query, export, and deletion.
6. **Material scale extraction:** separate control-plane services or queues
   require the measured CHILL-57 trigger, a dual-run reconciliation plan, and
   rollback evidence; tenancy contracts do not change at extraction.

## Consequences and verification

The internal MVP does not acquire billing, global routing, or provider
dependencies. The future external boundary is nevertheless concrete enough
to review schemas and APIs before the first public tenant. Verification for
activation includes state-machine property tests, repeated/out-of-order
webhooks, quota races, region cutover and rollback, suspension/reactivation,
support-grant expiry, complete regional deletion, and audit reconciliation.
CHILL-40 owns the threat review; CHILL-56 owns lifecycle and meter SLOs;
CHILL-57 owns measured extraction triggers.
