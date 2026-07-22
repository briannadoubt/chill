# Integrating Chill in Node.js, Deno, and Bun

`@chill-observability/runtime` instruments a host process. Its canonical source family is `server`; identify the runtime with OTLP resources (`service.name`, `process.runtime.name`, `process.runtime.version`, and `os.type`), not with another `source.platform` value.

## Install and configure

```sh
npm add @chill-observability/runtime
# Bun: bun add @chill-observability/runtime
```

```ts
import { createNodeRuntime } from "@chill-observability/runtime/node";
// Deno: createDenoRuntime from /deno. Bun: createBunRuntime from /bun.

const chill = createNodeRuntime({
  serviceName: "cats-worker",
  endpoint: "https://telemetry.internal.example/v1/logs",
  sdkKey: process.env.CHILL_API_KEY!,
  policyVersion: process.env.CHILL_POLICY_VERSION!,
  storePath: "/var/lib/cats/chill/records.ndjson",
  trustedOrigins: ["https://api.internal.example"],
  annotationKeys: ["account.tier", "job.kind"],
  maxAnnotations: 16, maxQueue: 1_000, flushIntervalMs: 10_000,
});
await chill.setConsent("granted");
await chill.start();
```

Choose a process-private writable queue. It retains already-filtered records for retry and acknowledges only successful OTLP responses, so delivery is at-least-once. The credential is held in the in-memory runtime configuration and is never serialized into records or the queue. Setting consent to `denied` aborts active export, purges the queue, and prevents later capture.

Populate `CHILL_POLICY_VERSION` from the authenticated
`/v1/chill/collection-state` endpoint before starting. Chill rejects records
that claim a stale privacy policy version.

## Use semantic events, activities, and trusted HTTP

There is no unbounded `track(name, properties)` API. Names are stable semantics; values enter only through allowlisted, bounded primitive annotations.

```ts
await chill.activity("billing.invoice", async () => {
  await chill.record({ name: "billing.invoice_sent", kind: "action",
    annotations: { "account.tier": "internal", "job.kind": "invoice" } });
  await chill.fetch("https://api.internal.example/invoices", { method: "POST" });
});
```

`activity` establishes async-local trace context. `chill.fetch` adds `traceparent` only for an exact `trustedOrigins` match; it never forwards credentials, arbitrary/received baggage, annotations, URL data, headers, or bodies. Re-evaluate redirect destinations before using another client for propagation. Arguments/results, messages, paths, environment variables, exception messages, and stacks are never automatically captured.

## Permissions and shutdown

```sh
deno run --allow-read=/var/lib/cats/chill --allow-write=/var/lib/cats/chill \
  --allow-net=telemetry.internal.example,api.internal.example app.ts
```

Node and Bun need equivalent sandbox/container filesystem and egress access. Drain only on a controlled process stop:

```ts
const stop = () => { void chill.stop(); };
process.once("SIGTERM", stop); process.once("SIGINT", stop);
```

## Troubleshooting and limits

- No records: grant consent before `record`; use declared names and allowlisted keys.
- Queue grows: check endpoint reachability/acceptance; retryable failures persist intentionally.
- No trace header: call `chill.fetch` inside an activity and configure the exact origin.

The pinned supported matrix is [platform support](platform-support.md). Native crash dumps, exactly-once delivery, arbitrary HTTP-client patching, and automatic payload capture are unsupported.
