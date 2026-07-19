# ADR-0023: Tenant access is credential-typed, current, and fail-closed

Status: accepted

## Decision

Every trusted request scope originates from an authenticated credential, never
from a client-supplied tenant identifier. Chill uses three deliberately
disjoint opaque credential types:

- `ch_sk_` SDK keys bind ingestion to one organization, project, environment,
  and data source, with only `ingest:otlp` and `ingest:replay` scopes;
- `ch_sv_` service credentials bind an internal worker to one organization,
  project, and environment plus an explicit capability set;
- `ch_us_` user sessions bind a verified identity to one active organization
  membership and resolve that membership's current role on every use.

All credentials contain random prefix and secret material. Chill returns the
raw credential exactly once and persists only the prefix and an
HMAC-SHA-256 digest keyed by an external pepper. Credential parsers reject one
type when another type is expected. Authentication checks credential status,
expiry, account and tenant status, and the exact resource hierarchy before
setting PostgreSQL tenant context.

## Identity boundary

`IssueUserSessionAfterIdentityVerification` is a narrow handoff, not an
identity provider. Its caller must validate the upstream provider's signed
assertion, issuer, audience, nonce, and freshness before passing the stable
issuer/subject pair to Chill. Chill then resolves that pair through
`control.user_identities` and requires an active user, membership, and
organization. A later SSO or passkey adapter must preserve this boundary; it
must not issue a session from an email address or user ID supplied by an
untrusted request.

Session authentication reads the current membership role rather than copying
authorization into a long-lived token. Suspending a user or membership,
changing a role, deleting a tenant, revoking a session, or rotating it is
effective on the next request.

## Roles and capabilities

The initial role matrix is intentionally small and closed:

| Role | Control read | Control write | Manage credentials | Read data | Delete data |
|---|---:|---:|---:|---:|---:|
| owner | yes | yes | yes | yes | yes |
| admin | yes | yes | yes | yes | yes |
| developer | yes | no | yes | yes | no |
| analyst | yes | no | no | yes | no |
| viewer | yes | no | no | no | no |

Unknown roles, capabilities, and service scopes fail closed. The existing
last-active-owner database invariant prevents an organization from becoming
ownerless. High-impact owner-only distinctions, invitation policy, and custom
roles require a new reviewed contract rather than ad hoc string checks.
Any capability delegated to a service credential must also be present in the
issuing user's current role, preventing credential management from becoming a
route to privilege escalation.

## Rotation, revocation, and audit

Rotation is a single tenant transaction: lock the active predecessor, insert a
new random credential with identical trusted scope and expiry, revoke the
predecessor, link both IDs, and append an audit record. There is no silent
overlap window. Revocation updates durable status immediately. User sessions
can self-rotate or self-revoke; credential managers can rotate and revoke SDK
and service credentials within their own tenant.

The append-only audit log records session creation, rotation, and revocation,
plus SDK and service credential creation, rotation, and revocation. It stores
IDs, scope metadata, actor, reason, and timestamps but never raw credentials or
digests.

## Isolation and resource limits

PostgreSQL row-level security remains the final control-plane boundary.
Application transactions set exactly one organization through a local
transaction setting, and tenant tables enforce that setting in `USING` and
`WITH CHECK` policies. Project and environment foreign keys make a mixed-tenant
resource tuple invalid even for privileged setup code. Security-definer lookup
functions expose only the minimum digest and trusted scope needed before tenant
context exists; runtime roles cannot read around RLS.

The integration suite creates two real organizations using a runtime database
role limited to `chill_app`. It proves that one tenant cannot query another's
projects, issue a credential for another tenant's environment, or rotate the
other tenant's SDK key. It also proves live role changes, least privilege,
typed credential rejection, rotation invalidation, revocation, and absence of
plaintext secret columns. Ingestion retains its transactionally enforced
per-environment request, record, and replay-byte quotas; query execution retains
its concurrency, scan, memory, time, and result limits.

## Evolution

The frugal internal deployment can use the built-in email identity mapping only
behind a trusted adapter. External SaaS requires a production identity provider,
MFA and recovery policy, CSRF-safe browser session transport, login abuse
limits, credential-notification workflows, and operator break-glass controls.
Those additions must keep opaque revocable sessions and database-enforced tenant
scope rather than replacing them with bearer claims that remain authoritative
after membership changes.
