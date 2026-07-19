# Chill browser SDK

The browser package captures semantic behavior, tracing, and source-masked structural replay without reading form values or page text. It has no server runtime: all Chill application backend logic remains in Rust.

```ts
import { ChillBrowser, annotationKey, instrumentDOM, instrumentFetch, startReplay } from "@chill-observability/browser";

const client = new ChillBrowser({
  endpoint: "https://chill.internal",
  sdkKey: "sdk_live_…",
  policyVersion: "internal-v1",
  consent: "granted",
  trustedTraceOrigins: ["https://api.internal"],
  storage: localStorage,
});

instrumentDOM(client);
globalThis.fetch = instrumentFetch(client, ["https://api.internal"]);
startReplay(client);
```

Declare UI semantics using `data-chill-page`, `data-chill-action`, `data-chill-impression`, `data-chill-form`, and JSON `data-chill-annotations`. Outer annotations win. React consumers use the `@chill-observability/browser/react` entry point.
