# Integrating Chill with Tauri 2

The Rust host uses `sdk/tauri` and `source.platform = server`; its webview uses the browser SDK plus `@chill-observability/tauri` and `source.platform = web`. Use resources—`service.name`, runtime/OS details, and `chill.desktop.framework = tauri`—to identify the actual runtime.

## Configure the host

```toml
[dependencies]
chill-observability = "0.1"
chill-tauri = "0.1"
tauri = "2"
```

```rust
use chill_tauri::{Correlator, TrustedWebview};

let correlation = Correlator::new([TrustedWebview {
    origin: "https://app.example.test".into(), label: "main".into(),
}]).command("adopt_cat", "cats.adopt");
```

Keep the host queue private and credentials memory-only. It is bounded and at-least-once: redacted records survive retryable failures and are acknowledged only after OTLP succeeds. Consent is enforced before IDs, serialization, persistence, and network work; denial cancels supported export, purges the queue, and makes capture a no-op.

## Explicit metadata-only commands

Declare a low-cardinality semantic name at each command boundary; do not use wildcard instrumentation.

```rust
#[tauri::command]
async fn adopt_cat(cat_id: String) -> Result<(), String> {
    adopt(cat_id).await
}
```

The dedicated metadata envelope has only a version marker, valid W3C `traceparent`, optional bounded public baggage, and declared command name. It is not derived from payload data. Chill never enumerates, clones, stringifies, hashes, logs, stores, or annotates command arguments/results. Missing/invalid metadata starts a local trace and bounded diagnostic without rejecting the command.

## Configure the webview

```ts
import { invoke } from "@tauri-apps/api/core";
import { invokeWithCarrier } from "@chill-observability/tauri";
await invokeWithCarrier(invoke, "adopt_cat", { catId: "application-owned-value" }, carrier);
```

Set up DOM collection, consent, browser storage, and web-fetch trust separately using [the browser guide](integrating-web.md). Host propagation uses the Rust exact-origin list; neither side forwards credentials, arbitrary/received baggage, annotations, URLs, headers, or IPC payloads. Call the host shutdown once on application exit. Grant filesystem access only to its queue and egress only to the OTLP endpoint/trusted origins; Tauri capabilities should expose narrow commands, not raw IPC.

Tauri 2 is supported. If records are missing, verify consent, command mapping, and queue/endpoint access; if traces do not join, verify both sides wrap the same command. Plugin-payload inspection, UI/pixel/text capture, native crash dumps, transparent command monkey-patching, and exactly-once delivery are unsupported.
