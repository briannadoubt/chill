# @chill-observability/tauri

Tauri webview correlation for Chill. The webview remains a `web` source via the
browser SDK. The Rust host is a `server` source and should identify Tauri with
`chill.desktop.framework=tauri` resource/instrumentation metadata.

Use a declared semantic name rather than raw command names and carry metadata
beside opaque arguments:

```ts
await invokeWithCarrier(invoke, "save_account", applicationArguments, {
  version: 1, traceparent, semanticName: "account.save",
});
emit("account_changed", eventEnvelope(carrier, applicationPayload));
```

`invokeWithCarrier` only JSON-serializes the bounded carrier into an
`x-chill-carrier` invoke header. Event
payloads are explicitly wrapped alongside the carrier. Neither helper reads,
enumerates, formats, hashes, measures, logs, or persists application data.
Never put a Chill credential in a webview; the host Rust SDK owns export.
Carrier metadata is correlation only and never authorizes a caller, tenant, or
consent state.

The host must configure the exact trusted webview origin and label (for example,
`https://app.example.test` and `main`) in `chill-tauri`; matching only one is
not sufficient. Baggage is denied by default. Allow each public baggage key in
both the host `Correlator::allow_baggage_key` and renderer `baggageAllowlist`.
