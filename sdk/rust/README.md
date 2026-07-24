# chill-observability

Standalone Rust 1.95 / edition-2024 portable SDK. It emits only declared typed
semantic events, actions, activities, and process session lifecycle facts.
Collection is denied by default; call `set_consent(Consent::Granted)` before
capture, then call `start_session()` before application events or activities.
Values are accepted only through registered, bounded annotation keys.

`Config` requires an application-selected queue directory. Queue entries are
atomic, redacted JSON records with UUIDv7 record IDs; credentials stay solely in
memory. The queue is at-least-once: peek/export, then acknowledge only a 2xx
OTLP response. The `otlp_json` helper produces the `/v1/logs` OTLP/HTTP JSON
shape accepted by Chill. Rust processes report `source.platform=server` and
Rust/OS resource attributes.

Set `Config::credential` in memory before `flush().await` or `shutdown().await`.
Set `Config::policy_version` to the active version returned by the authenticated
`/v1/chill/collection-state` endpoint; new Chill environments default to
`privacy-v1`.
The SDK sends bounded OTLP/HTTP JSON batches with a Bearer authorization header;
only a 2xx response acknowledges queue files. Failed or non-2xx requests retain
their original UUIDv7 record IDs for retry.

See `examples/linux-daemon.rs` and `examples/desktop-process.rs`. HTTP trace
context is injected only for exact origins in `trusted_origins`; invalid W3C
headers are rejected. `install_panic_hook` chains the previous hook and emits
only `process.panic`, never panic text, paths, or stack data.
