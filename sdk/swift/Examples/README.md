# Chill Apple examples

These applications are compile-checked consumer packages. They intentionally
contain no imperative tracking calls:

- `ChillSwiftUISample` uses view declarations and Swift macros.
- `ChillUIKitSample` uses native controller, control, and macro declarations.

Both configure the durable OTLP exporter once at launch. Set
`CHILL_OTLP_ENDPOINT` and `CHILL_API_KEY` to point a sample at a collector; the
default endpoint uses the reserved `.invalid` domain. For a loopback HTTP
collector only, also set `CHILL_ALLOW_INSECURE_LOCALHOST=1`; deployed endpoints
must use HTTPS.

The [Apple integration guide](../../../docs/integrating-swift.md) explains
installation, runtime configuration, declarations, macros, outer-first
resolution, privacy and consent, OpenTelemetry interoperability, replay,
migration, and troubleshooting. The source files here are its compile-checked
companion examples.

From the repository root, run `scripts/validate-swift-sdk.sh` to compile the
examples alongside the SDK, conformance report, tests, and release benchmarks.
