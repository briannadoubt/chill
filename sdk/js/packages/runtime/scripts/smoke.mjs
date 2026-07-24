const runtime = process.argv[2] ?? "node";
const entry = process.argv[3] ?? new URL(`../dist/${runtime}.js`, import.meta.url).href;
const { createRuntime } = await import(entry);
const client = createRuntime({ serviceName: "packed-smoke" });
await client.setConsent("granted");
await client.activity("smoke.run", () => client.record({ name: "smoke.ok" }));
if (!client.current && runtime === "node") throw new Error("runtime factory failed");
console.log(`${runtime} smoke ok`);
