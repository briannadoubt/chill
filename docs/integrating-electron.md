# Integrating Chill with Electron

Use `@chill-observability/electron` in the main process and the browser SDK plus `@chill-observability/electron/renderer` in each renderer. Main uses `source.platform = server`; renderers use `web`. Resources carry `service.name`, runtime/OS details, and `chill.desktop.framework = electron`.

## Main setup

```sh
npm add @chill-observability/electron @chill-observability/browser
```

```ts
import { app } from "electron";
import { ChillElectronMain } from "@chill-observability/electron/main";
await app.whenReady();
const chill = new ChillElectronMain({ port: hostPort });
chill.observeApp(app);
```

Use user-data storage inaccessible to renderers; credentials stay memory-only. The bounded queue is at-least-once. Consent denial purges it and stops further collection. Lifecycle facts never include UI contents, crash messages/stacks, paths, credentials, or payloads.

## Metadata-only IPC

Map each concrete channel to one stable semantic name; wildcards are unsupported.

```ts
// main.ts
chill.registerIpc(ipcMain, { channel: "cats:adopt", semanticName: "cats.adopt",
  handler: (_event, catId) => adoptCat(catId as string) });
// preload.ts: expose a narrow API, never raw ipcRenderer
const renderer = new ChillElectronRenderer(ipcRenderer, { port: rendererPort, browser });
contextBridge.exposeInMainWorld("cats", { adopt: (catId: string) =>
  renderer.invoke("cats.adopt", "cats:adopt", catId) });
```

Only a version marker, valid `traceparent`, bounded public baggage, and command name cross in Chill metadata. Arguments/results are untouched: adapters do not enumerate, clone, stringify, hash, log, store, or annotate them. Invalid/absent metadata creates a local trace and bounded diagnostic, never rejects application IPC.

Configure renderer DOM telemetry separately using [the browser guide](integrating-web.md). Flush/shut down the host-owned runtime during normal quit. If correlation fails, check that preload and main declare the same concrete channel/name. Electron support requires a maintained release with a supported embedded Node; automatic IPC interception, native crash dumps, UI text/pixel capture, and exactly-once delivery are unsupported.
