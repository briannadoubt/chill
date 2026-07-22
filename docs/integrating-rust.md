# Integrating Chill in Rust

The portable Rust client is in `sdk/rust`. It emits canonical `source.platform = server`; use resources (`service.name`, `process.runtime.name`, `process.runtime.version`, `os.type`) to identify Rust and the host OS.

## Setup

```toml
[dependencies]
chill-observability = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
```

```rust
use chill_observability::*;
use std::collections::BTreeMap;

let mut config = Config::new(ServiceName::try_from("cats-worker")?,
    "/var/lib/cats/chill", "https://telemetry.internal.example/v1/logs");
config.consent = Consent::Granted;
config.credential = Some(std::env::var("CHILL_API_KEY")?);
config.policy_version = std::env::var("CHILL_POLICY_VERSION")?;
config.trusted_origins.insert("https://api.internal.example".into());
let chill = Client::new(config)?;
chill.start_session();
```

Use a process-private queue directory. The bounded store persists redacted records atomically, retains original IDs on retry, and only acknowledges successful OTLP responses. Consent is checked before record IDs, serialization, disk, or network work; denial purges the queue and stops capture. Keep credentials memory-only.

Populate `CHILL_POLICY_VERSION` from the authenticated
`/v1/chill/collection-state` endpoint at startup and use its `policy_version`;
the backend rejects stale policy versions rather than silently weakening policy.

## Semantics, propagation, and shutdown

```rust
chill.event(SemanticName::try_from("billing.invoice_sent")?, BTreeMap::new());
// Use the client activity wrapper around one async operation; inject traceparent
// only after an exact trusted-origin check in the application's HTTP client.

chill.install_panic_hook();
tokio::signal::ctrl_c().await?;
chill.end_session();
chill.shutdown().await?;
```

Activities use Tokio task-local context. Trusted requests inject only valid `traceparent` plus explicitly allowlisted bounded baggage for exact origins; redirects must be rechecked. Trace context is never authorization, identity, tenancy, or consent. Chill never auto-captures arguments, results, errors, URLs, headers, bodies, paths, environment variables, panic messages, or stacks; failures use stable reason codes.

Grant write access only to the queue and egress only to OTLP/trusted origins. Linux GNU `x86_64` and `aarch64` are supported; musl, Windows, GUI automation, native crash dumps, and exactly-once delivery are not.
