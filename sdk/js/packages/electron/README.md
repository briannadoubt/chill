# @chill-observability/electron

Use explicit, developer-declared semantic names.  Application payloads, channel names, URLs, titles, notes, messages and stacks are never inspected or recorded.

```ts
import { ChillElectronRenderer } from "@chill-observability/electron/renderer";

const chill = new ChillElectronRenderer(ipcRenderer, { port, browser, traceparent });
await chill.invoke("preferences_saved", "preferences:save", preferences);
```
