# Platform support

Chill is pre-1.0. "Supported" means the repository contains a public package,
an integration guide or compile-checked example, deterministic contract tests,
and a pinned CI gate. It is not an uptime SLA or long-term compatibility
promise.

| Platform or runtime | Package | Status | Canonical source | Coverage |
| --- | --- | --- | --- | --- |
| Apple platforms | `sdk/swift` | Complete first vertical slice | `apple` | SwiftUI, UIKit/AppKit, lifecycle, networking, durable export, structural replay |
| Android | `sdk/android` | Active | `android` | Compose, Views, lifecycle, networking, durable export, structural replay |
| Browser and React | `sdk/web` | Active | `web` | DOM/React semantics, navigation, fetch/XHR, durable export, structural replay |
| Rust on Linux GNU | `sdk/rust` | Active | `server` | Semantic events/activities, async context, lifecycle/panic hooks, durable OTLP export |
| Tauri 2 host | `sdk/tauri` | Active | `server` | Rust lifecycle and metadata-only command correlation |
| Tauri webview | browser SDK + `@chill-observability/tauri` | Active | `web` | Browser behavior plus metadata-only command correlation |
| Electron main process | `@chill-observability/electron` | Active | `server` | App/window/updater/crash lifecycle and IPC handlers |
| Electron renderer | browser SDK + `@chill-observability/electron/renderer` | Active | `web` | Browser behavior plus metadata-only IPC invokes |
| Node.js | `@chill-observability/runtime` | Active | `server` | Async context, semantic events/activities, trusted fetch, bounded durable export |
| Deno | `@chill-observability/runtime` | Active | `server` | Same runtime surface through Deno's Node/Web compatibility, with required file/network permissions |
| Bun | `@chill-observability/runtime` | Active | `server` | Same runtime surface through Bun's Node/Web compatibility |

The source family is intentionally coarse. Runtime and framework identity is
carried in OpenTelemetry resource attributes such as `process.runtime.name`,
`process.runtime.version`, `os.type`, `service.name`, and
`chill.desktop.framework`.

## Compatibility policy

- Rust uses the repository-pinned stable toolchain. Linux release checks cover
  `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.
- Tauri support targets major version 2 and uses explicit semantic command
  wrappers; arbitrary plugin or payload inspection is unsupported.
- Electron support targets maintained Electron releases whose embedded Node
  runtime satisfies the runtime package's engine requirement.
- Node, Deno, and Bun versions are pinned in `ci/toolchains.json`; a version is
  supported only after its compatibility suite is green.
- Windows GNU/MSVC, Linux musl, and additional native GUI toolkits are
  best-effort until they gain their own pinned CI targets and examples.
- React Native and Flutter remain out of scope.

## Privacy boundary

Desktop and runtime adapters capture stable semantic names, lifecycle states,
outcomes, timing, and explicitly registered bounded annotations. They do not
capture IPC arguments or results, UI text, message bodies, URLs, headers,
environment variables, paths, exception messages, or stack traces. Trace
headers are injected only for exact trusted origins, and IPC correlation uses
a dedicated metadata envelope that is never derived from application payloads.
