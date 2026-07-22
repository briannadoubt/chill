# chill-tauri

`chill-tauri` correlates declared Tauri 2 commands and events with Chill traces.
It uses `source.platform=server` in the host; instrument the webview with the
browser SDK (`source.platform=web`). Set `chill.desktop.framework=tauri` as a
host resource/instrumentation attribute.

The core has no Tauri or GUI dependency. Configure the exact trusted webview
origin and label, and explicitly map raw command/event names to semantic names:

```rust
let correlation = chill_tauri::Correlator::new([chill_tauri::TrustedWebview {
    origin: "https://app.example.test".into(), label: "main".into(),
}]).command("save_account", "account.save");
```

Pass the carrier beside application arguments. Never pass a credential to the
webview: the host's public Chill Rust SDK owns collection and export. Invalid,
absent, or untrusted carriers start local context and supply only a bounded
reason code; correlation never grants authorization, tenancy, or consent.
