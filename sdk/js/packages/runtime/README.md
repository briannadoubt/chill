# @chill-observability/runtime

ESM-only, privacy-first server instrumentation for Node 22, Deno 2, and Bun 1. Collection is denied until explicitly granted. An export endpoint must be the complete `https://host/v1/logs` OTLP endpoint and requires an SDK key; the key remains in memory and is sent only as `Authorization: Bearer`.

```ts
import { createNodeRuntime } from "@chill-observability/runtime/node";
const chill = createNodeRuntime({
  serviceName: "orders", endpoint: "https://ingest.example/v1/logs", sdkKey: "chill_sdk_…",
  policyVersion: "privacy-v1", installationId: "9f22f5c1-c67f-4c2d-9f04-0b1fc3000001",
  annotationKeys: ["http.method"], trustedOrigins: ["https://api.internal"]
});
await chill.setConsent("granted");
await chill.activity("order.submit", () => chill.record({ name: "order.created", kind: "action", annotations: { "http.method": "POST" } }));
```

`record({ name, kind?, annotations? })` emits an instant `event` (or `action`) whose subject equals its record ID. `activity(name, fn, parent?)` emits paired `start` and `end` activity records with one UUIDv7 subject and one child trace. Each accepted record is exported as OTLP/HTTP JSON with Chill V1 identity, source, context, privacy, and clock attributes; retries preserve its UUIDv7 identity. Only registered annotations are retained. URLs, paths, request headers/bodies, errors, stacks, environment, and credentials are never recorded. `fetch` propagates a strict W3C traceparent only to exact trusted origins. Use `/deno` or `/bun` imports for other hosts.

Set `policyVersion` to the active version returned by the authenticated
`/v1/chill/collection-state` endpoint. New Chill environments default to
`privacy-v1`.
