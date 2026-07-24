const deno = globalThis.Deno;
const args = deno?.args ?? process.argv.slice(2);
const [host = "node", entry = "../dist/node.js"] = args;
const readEnvironment = (name) => deno?.env.get(name) ?? process.env[name];
const endpoint = readEnvironment("CHILL_OTLP_ENDPOINT");
const sdkKey = readEnvironment("CHILL_API_KEY");
if (!endpoint || !sdkKey) {
  throw new Error("CHILL_OTLP_ENDPOINT and CHILL_API_KEY are required");
}

const module = await import(entry);
const createRuntime =
  module.createNodeRuntime ??
  module.createDenoRuntime ??
  module.createBunRuntime ??
  module.createRuntime;
const runtime = createRuntime({
  serviceName: `chill.portable.${host}.smoke`,
  runtimeName: host,
  endpoint,
  sdkKey,
  policyVersion: readEnvironment("CHILL_POLICY_VERSION") ?? "privacy-v1",
});

await runtime.setConsent("granted");
await runtime.start();
await runtime.record({
  name: "portable.javascript.ready",
});
await runtime.record({
  name: "portable.javascript.action",
  kind: "action",
});
await runtime.activity("portable.javascript.activity", async () => {
  await Promise.resolve();
});
await runtime.stop();

if (runtime.diagnostics.some((diagnostic) => diagnostic.detail === "flush_failed")) {
  throw new Error("portable JavaScript export was not acknowledged");
}
console.log(`portable-${host}-ingest-ok`);
