# ADR-0016: authenticated OTLP enters a durable tenant-stamped inbox

## Status

Accepted.

## Decision

The initial `chilld` process accepts stable OTLP logs, traces, and metrics over
both OTLP/HTTP and OTLP/gRPC. HTTP supports `application/x-protobuf` in
production and `application/json` for conformance/debugging. Gzip is decoded
through both compressed- and expanded-byte limits. Both transports call one
bounded admission service; neither transport can bypass authentication,
validation, quotas, idempotency, or durable acknowledgement.

An SDK key is supplied once as `Authorization: Bearer <key>` or
`X-Chill-SDK-Key: <key>`. Supplying both is rejected. The service authenticates
the HMAC-only key through the control plane and stamps organization, project,
environment, data-source, and SDK-key IDs from that trusted result. Tenant IDs
from client headers or OTLP resources have no authority.

Accepted requests are inserted into `ingest.inbox` in PostgreSQL before success
is returned. This is the intentionally replaceable, inexpensive boundary until
object-storage micro-batching exists; PostgreSQL is not the analytical lake.
The same transaction validates active Chill schema references and reserves an
exact environment minute bucket plus a UTC daily replay-byte bucket.

## Idempotency and acknowledgement

`Idempotency-Key` is accepted when it contains 1–128 bounded ASCII characters.
Without it, Chill behavior logs derive a key from sorted `chill.record.id`
values; generic OTLP derives one from the payload digest; replay derives one
from the chunk ID. A per-key transaction advisory lock prevents concurrent
duplicates from consuming quota twice.

- same key and same SHA-256 payload: `200`, original inbox receipt, duplicate;
- same key and different payload: `409` / gRPC `FailedPrecondition`;
- success is returned only after the inbox transaction commits.

## Limits and retry semantics

The initial uncompressed limits are 4 MiB per OTLP request, 8 MiB per replay
chunk, 10,000 records, and 32 concurrent admissions. Database quotas further
bound requests/minute, records/minute, and replay bytes/day.

| Condition | HTTP | gRPC | Retry |
|---|---:|---|---|
| malformed/schema invalid | 400 | `InvalidArgument` | fix request |
| missing or bad SDK key | 401 | `Unauthenticated` | fix key |
| missing key scope | 403 | `PermissionDenied` | fix configuration |
| expanded/compressed too large | 413 | `ResourceExhausted` | split request |
| idempotency conflict | 409 | `FailedPrecondition` | new/correct key |
| quota or process backpressure | 429 | `ResourceExhausted` | `Retry-After` / `RetryInfo` |
| database unavailable | 503 | `Unavailable` | retry with backoff |

OTLP errors use `google.rpc.Status` in the request encoding. Successful OTLP
responses use the signal's standard empty export response.

## Replay binary route

`POST /v1/chill/replay` accepts
`application/vnd.chill.replay.v1+octet-stream` with the encrypted
`CHILLRP1` envelope and bounded headers for replay/chunk/session/boot identity, source
and monotonic time, codec, SHA-256 digest, and optional W3C `traceparent`.
Replay requires `ingest:replay`; OTLP requires `ingest:otlp`. The server verifies
the envelope magic and digest without decrypting potentially sensitive frames.
Key-envelope and playback processing remain downstream concerns; ingest never
places replay bytes inside OTLP logs.

## Consequences

- One small deployment has crash-safe acknowledgement without Kafka.
- PostgreSQL growth is bounded later by the inbox publisher/compactor.
- Authentication currently performs a control-plane lookup per request; a
  bounded revocation-aware cache may be added only with explicit stale-key
  tests.
- TLS termination and production deployment policy belong to CHILL-27; local
  `chilld` supports h2c so HTTP and gRPC share one listener.
