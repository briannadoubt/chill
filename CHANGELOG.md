# Changelog

Chill uses coordinated repository versions from `VERSION`. The project is
pre-1.0; release notes call out contract and compatibility changes explicitly.

## Unreleased

### Added

- A source-only Unity 2022.3+ package with consent-gated scene, action, event,
  impression, and activity semantics plus bounded at-least-once OTLP delivery.
- A standalone portable Rust SDK with Linux support, durable OTLP/HTTP export,
  semantic events/actions/activities, async trace context, trusted propagation,
  lifecycle and privacy-safe panic reporting.
- An ESM JavaScript runtime for pinned Node, Deno, and Bun releases.
- Payload-blind Electron and Tauri 2 adapters for host/renderer trace
  correlation and stable lifecycle facts.
- Portable V2 conformance, release-budget contracts, support matrix,
  integration guides, reproducible source artifacts, and live-ingestion smoke
  coverage.

### Compatibility

- Canonical behavior V1 and its `conformance/v1` digest remain unchanged.
- Portable extensions use `conformance/v2` while host records retain
  `source.platform=server` and webviews retain `source.platform=web`.
