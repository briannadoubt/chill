# ADR-0028: Scale only at measured extraction triggers

- Status: Accepted
- Date: 2026-07-17

## Context

Chill 0.1 is a Rust modular monolith with PostgreSQL, S3-compatible immutable
objects, and embedded DuckDB. Queues, distributed databases, and regional
services add failure modes and operator load. They are justified only when a
measured bottleneck persists after query tuning, bounded concurrency, vertical
capacity, and workload scheduling.

## Decision

Each extraction needs seven consecutive days of production evidence, an owner,
a load-test reproduction, a contract-preserving migration, a rollback, and a
30-day cost comparison. No trigger is “traffic is growing.”

| Boundary | Trigger after ordinary tuning | Migration | Cutover and rollback |
| --- | --- | --- | --- |
| PostgreSQL inbox to durable log | p99 durable admission >250ms or inbox >15m for 3 days while DB CPU >70% and storage IOPS is limiting | dual-write versioned receipt IDs to a regional Kafka-compatible log; PostgreSQL remains idempotency ledger until parity | shadow consume, compare receipt/count/digest, then move workers; disable log writes and replay from PostgreSQL on mismatch |
| Ingest process extraction | ingest consumes >60% host CPU or memory and causes query/lifecycle SLO burn at the smallest cost-effective vertical size | move the unchanged Rust ingest crate behind the same edge; keep PostgreSQL transaction contract | weighted traffic with identical response fixtures; route back to monolith without data conversion |
| Lake table format | manifest planning >500ms p95, >100k live files/environment, or compaction consumes >25% weekly compute | publish Iceberg-compatible metadata beside existing manifests; Parquet objects and canonical schema remain unchanged | dual-read verification, atomic catalog pointer; restore old manifest pointer |
| Distributed query | bounded reference plans exceed 8s p99 for 7 days after compaction/cache tuning or scan demand blocks ingest | introduce a tenant-scoped Rust query coordinator and managed engine using typed plans only | shadow plans and compare columns/rows; send all plans back to embedded DuckDB |
| Regional deployment | p95 network RTT to the nearest allowed region >150ms for 20% of production users, residency requires it, or regional availability dominates budget | deploy independent regional cells; organization home region is authoritative and movement uses ADR-0027 state machine | migrate one tenant with digest reconciliation; return routing and authority to source cell before finalization |
| Dedicated replay processing | replay CPU/object bandwidth causes replay or ingest burn for 3 days, or backlog freshness >5m at 70% replay quota | extract the Rust replay worker with the same encrypted chunk/index contract | dual-index metadata, compare gaps/digests, then switch leases; return leases to monolith |

## Measurement and governance

The evidence bundle contains SLO panels, host/DB/object metrics, tenant-safe
volume histograms, flame graphs, cost projection, load-test commands, schema
and protocol versions, failure injection, and rollback timing. Product volume
alone is insufficient. A trigger opens a design ticket; it does not authorize
immediate migration.

Extraction preserves canonical IDs, typed plans, immutable object keys,
idempotency semantics, privacy policy versions, audit records, and the rule
that all server-side Chill logic is Rust. New infrastructure may transport or
store those contracts but may not become an alternate business-logic runtime.

## Consequences

The monolith can remain deliberately boring while measurements are healthy.
When it stops being healthy, each first extraction has a rehearsed seam and a
reversible cutover instead of a speculative rewrite.
