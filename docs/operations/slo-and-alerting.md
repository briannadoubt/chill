# Chill 0.1 service levels and operating response

These objectives cover the single-node internal deployment. They are product
commitments measured over rolling 30-day windows, not host uptime claims. A
single operator owns the page during 0.1; every page therefore names one safe
first action and avoids alerts that cannot change an outcome.

| Capability | SLI and objective | Window / budget | Measurement |
| --- | --- | --- | --- |
| OTLP ingest availability | durable acknowledgements / valid attempts >= 99.9% | 30 days / 43m 49s equivalent | `chill_http_requests_total{surface="ingest"}`; 4xx policy rejects are excluded, 429 is unavailable |
| Replay availability | durable replay acknowledgements / valid attempts >= 99.5% | 30 days / 3h 39m | `surface="replay"` plus an encrypted canary chunk |
| Accepted-data durability | acknowledged canary records visible after restart = 100%; no receipt gaps | every release and weekly | authenticated write/read canary and restore drill |
| Data freshness | p99 acknowledgement-to-committed-manifest <= 5m | rolling 24h | tenant-scoped canary compares receipt and query-visible time |
| Query latency | p95 <= 2s and p99 <= 8s for the reference bounded plans | rolling 7d | `surface="query"` histogram and scheduled reference plans |
| SDK delivery | >= 99.5% of accepted batches delivered within 15m when online | rolling 7d by SDK/platform | privacy-safe SDK diagnostics aggregate; offline time excluded |
| Tenant isolation | zero unauthorized cross-tenant reads or writes | continuous | negative integration canary, RLS tests, and security event review |
| Cost | monthly infrastructure <= $150 and <= $0.25 per million accepted records | calendar month | invoice plus accepted durable receipts; alert at 70% and 90% |

The Rust process exposes low-cardinality request counters, latency histograms,
active work, and PostgreSQL pool gauges at the private `/metrics` endpoint. It
never labels by tenant, credential, path parameter, trace, or user. Freshness,
durability, SDK delivery, and isolation require authenticated synthetic probes;
deriving them from a privileged cross-tenant SQL scrape would weaken the RLS
boundary and is explicitly prohibited.

## Burn-rate policy

- Page when the 1-hour and 6-hour availability burn rates both exceed 14.4x
  and 6x respectively, or when the durability/isolation canary fails once.
- Create a same-day ticket when the 6-hour and 3-day burn rates exceed 3x and
  1x, query p99 exceeds 8 seconds for 30 minutes, or freshness p99 exceeds five
  minutes twice.
- Cost alerts are non-paging. SDK delivery alerts page only when two platforms
  or the production app are affected; a single SDK version creates a release
  ticket.
- Maintenance consumes budget unless it is a tested, announced disaster
  recovery operation. Empty traffic windows do not count as success.

## First response runbook

1. Confirm `/livez`, `/readyz`, and the external TLS probe. If readiness is
   down, stop deploy activity and inspect PostgreSQL/object-store health.
2. Compare `failed` versus `rejected` outcomes by bounded surface. A rejected
   surge usually means credentials, quota, body size, or a bad client release;
   do not scale the database before identifying the reason.
3. If active requests or latency climb with a saturated PostgreSQL pool, pause
   analytics and lifecycle work before ingest. If ingest remains unhealthy,
   roll back the application image with `upgrade.sh`.
4. For freshness failures, verify the inbox worker, then lake publication and
   object-store health. Never delete pending/leased rows to clear a queue.
5. For a durability or isolation failure, freeze releases, preserve logs and
   manifests, revoke affected credentials, and treat the event as a security
   incident. Restoration is into a new bucket/database until verification.
6. Record start/end, affected SLO, budget consumed, versions, safe action,
   evidence, and follow-up ticket. A page closes only after both short and long
   burn windows recover.

## Dashboard contract

The operator dashboard shows availability and burn rate first, then latency,
active work, pool saturation, canary freshness/durability, storage growth,
accepted record rate, cost projection, and current release. All panels include
freshness timestamps. Dashboards and alerts use the rules in
`deploy/observability/alerts.yml`; environment-specific receivers remain
outside the repository.
