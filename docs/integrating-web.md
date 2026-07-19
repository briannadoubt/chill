# Integrating the Chill browser SDK

Install `@chill-observability/browser`, create one `ChillBrowser` after the application has resolved collection consent, and provide browser storage for crash-safe delivery. The browser package is client-only; it sends OTLP/HTTP to the Rust ingest service.

```ts
import {
  ChillBrowser,
  instrumentDOM,
  instrumentFetch,
  instrumentNavigation,
  instrumentXMLHttpRequest,
  startReplay,
} from "@chill-observability/browser";

const chill = new ChillBrowser({
  endpoint: "https://telemetry.internal.example",
  sdkKey: import.meta.env.VITE_CHILL_SDK_KEY,
  policyVersion: "internal-v1",
  consent: "granted",
  sampleRate: 1,
  replaySampleRate: 0.1,
  storage: localStorage,
});

const stopDOM = instrumentDOM(chill);
const stopNavigation = instrumentNavigation(chill);
const stopXHR = instrumentXMLHttpRequest(chill, ["https://api.internal.example"]);
globalThis.fetch = instrumentFetch(chill, ["https://api.internal.example"]);
const stopReplay = startReplay(chill);
```

Use `data-chill-page`, `data-chill-action`, `data-chill-impression`, and `data-chill-form` on semantic elements. `data-chill-annotations` accepts a JSON object. An outer annotation is authoritative and collisions become SDK diagnostics. Form values, DOM text, accessibility labels, URLs, headers, bodies, pixels, and error messages are never captured by the automatic adapters.

React applications import `ChillProvider`, `ChillPage`, `ChillAnnotation`, and `useChillAction` from `@chill-observability/browser/react`. The provider does not require a framework-specific backend.

Only origins in the explicit trust list receive W3C `traceparent`; third-party origins receive no propagation headers. Call `flush()` during normal lifecycle checkpoints and `stop()` during controlled shutdown. Failed exports remain in the bounded local queue for retry.
