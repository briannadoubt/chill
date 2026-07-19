# OTLP V1 conformance fixtures

These files are exact, minified OTLP/HTTP JSON payloads. They are intentionally
checked in as golden wire fixtures so field spelling, ID encoding, enum values,
64-bit integer encoding, resource placement, scope placement, and attribute
ordering cannot drift unnoticed.

- `action-log.json` is the lossless event/log projection of
  `examples/behavior/v1/action-activated.json`.
- `activity-span.json` is the trace projection formed by
  `activity-started.json` and `activity-ended.json`.

The canonical behavior examples remain the source of truth. Update a golden
only after an intentional contract change and review both sides of the mapping.
