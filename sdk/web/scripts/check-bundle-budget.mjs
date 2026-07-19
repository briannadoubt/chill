import { stat } from "node:fs/promises";
const files = ["dist/index.js", "dist/runtime.js", "dist/dom.js", "dist/tracing.js", "dist/replay.js", "dist/storage.js", "dist/exporter.js", "dist/types.js"];
let bytes = 0; for (const file of files) bytes += (await stat(file)).size;
const maximum = 40 * 1024; if (bytes > maximum) throw new Error(`browser runtime ${bytes} bytes exceeds ${maximum}`);
console.log(`browser runtime ${bytes} bytes / ${maximum} byte budget`);
