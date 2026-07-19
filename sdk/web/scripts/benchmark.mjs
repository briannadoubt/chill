import { performance } from "node:perf_hooks";
import { webcrypto } from "node:crypto";
import { ChillBrowser } from "../dist/index.js";
if (!globalThis.crypto) Object.defineProperty(globalThis, "crypto", { value: webcrypto });
const client = new ChillBrowser({ endpoint: "https://localhost", sdkKey: "benchmark", policyVersion: "benchmark-v1", consent: "granted", maxBufferedRecords: 20_000, fetch: async () => new Response(null, { status: 200 }) });
const count = 10_000; const started = performance.now(); for (let index = 0; index < count; index += 1) client.action("benchmark.action", { index });
const perRecordMs = (performance.now() - started) / count; const maximumMs = 0.2;
if (perRecordMs > maximumMs) throw new Error(`${perRecordMs.toFixed(4)}ms per record exceeds ${maximumMs}ms`);
console.log(`${perRecordMs.toFixed(4)}ms per record / ${maximumMs}ms budget`);
