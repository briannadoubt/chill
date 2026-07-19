# ADR-0022: Data classification and source redaction are fail-closed

Status: accepted

## Decision

Chill uses the versioned catalog in
`policies/privacy/v1/data-classification.json` for identifiers, content, text,
network fields, device data, custom annotations, secrets, and credentials.
Every source is omitted unless a narrow rule says otherwise. Secrets and
credentials are never eligible. Arbitrary annotations are serialized only
when their exact semantic key appears in the active environment allowlist with
an annotation-eligible classification.

An allowlist entry describes the data; it does not weaken consent. Personal
data still requires an enabled capture class, purpose-specific policy,
retention, export, and deletion. Sensitive content is not annotation-eligible
because a scalar annotation has no safe source transform. It must be omitted
or represented by a purpose-built mask.

## Enforcement boundaries

The native SDK resolves outer-first annotations, intersects the resolved keys
with its configured allowlist, omits unsafe values, increments redaction
metadata, and sends only the surviving key/classification pairs to its sink.
The exporter serializes only values carrying that classification metadata, so
a hand-built unclassified snapshot cannot bypass the runtime boundary.

Ingest independently compares every annotation and classification against the
trusted active policy for the authenticated environment. Missing, extra,
mismatched, unsafe, or stale-policy declarations dead-letter the envelope.
Client tenant context and client policy claims are never trusted as authority.

Replay is structural. Text, form state, accessibility text, secure input, and
pixels become masks before admission to an in-memory replay buffer. Custom
drawing is blocked. Raw sensitive content must not exist temporarily for later
redaction.

## Developer experience

Developers keep declarative call sites such as
`.annotation("cat.id", value: cat.id)`. Classification belongs in the central
environment policy, where it is reviewable and consistent across platforms;
it is not repeated at every view. Unknown keys silently emit no user data and
produce redaction diagnostics rather than falling back to permissive capture.

## Evolution

Classification names and policy shape are versioned contracts. A new class,
source field, or less restrictive transform requires a schema version and
cross-platform conformance fixtures. Older SDKs and ingest workers continue to
fail closed on unknown values.
