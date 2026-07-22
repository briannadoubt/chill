import { createRuntime } from "@chill-observability/runtime";
const chill = createRuntime({ serviceName: "example", endpoint: "https://ingest.example/v1/logs", sdkKey: "chill_sdk_replace_me", policyVersion: "privacy-v1", annotationKeys: ["job.kind"] });
await chill.setConsent("granted");
await chill.activity("job.run", () => chill.record({ name: "job.finished", annotations: { "job.kind": "daily" } }));
