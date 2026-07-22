import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { AnnotationContext, ChillBrowser, annotationKey, instrumentActivity, parseTraceparent, traceparent } from "../src/index.js";

test("outer annotations remain authoritative and diagnose collisions", () => {
  const key = annotationKey<string>("experiment.variant");
  const context = AnnotationContext.empty().with(key, "control").mergeDescendant(AnnotationContext.empty().with(key, "treatment"));
  assert.equal(context.values.get(key.name), "control"); assert.equal(context.diagnostics[0]?.code, "annotation_collision");
});

test("consent gates capture and flushes OTLP without payload text", async () => {
  const dom = new JSDOM("", { url: "https://app.example/path" });
  Object.defineProperty(globalThis, "location", { configurable: true, value: dom.window.location });
  let body = "";
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "internal-v1", consent: "denied", fetch: async (_input, init) => { body = String(init?.body); return new Response(null, { status: 200 }); } });
  client.event("message.send", { event_class: "domain", emission: "observed" }); assert.equal(await client.flush(), 0);
  client.setConsent("granted"); client.event("message.send", { event_class: "domain", emission: "observed" }); assert.equal(await client.flush(), 2);
  assert.match(body, /message\.send/); assert.doesNotMatch(body, /entered_text/);
});

test("OTLP projection includes canonical web source, privacy, page, and allowlisted annotation attributes", async () => {
  let body = "";
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "privacy-v1", consent: "unknown", installationId: "12345678-1234-4234-8234-123456789abc", allowedAnnotationKeys: ["app.area"], fetch: async (_input, init) => { body = String(init?.body); return new Response(null, { status: 200 }); } });
  client.setContext(AnnotationContext.empty().with(annotationKey("app.area"), "console"));
  client.setConsent("granted"); client.startPage("overview"); client.event("console.open", { event_class: "lifecycle", emission: "observed" });
  assert.equal(await client.flush(), 3);
  const payload = JSON.parse(body) as { resourceLogs: Array<{ scopeLogs: Array<{ logRecords: Array<{ attributes: Array<{ key: string; value: unknown }> }> }> }> };
  const records = payload.resourceLogs[0]!.scopeLogs[0]!.logRecords;
  const attributes = new Map(records.flatMap(record => record.attributes.map(attribute => [attribute.key, attribute.value] as const)));
  assert.ok(attributes.has("chill.source.platform"));
  assert.ok(attributes.has("chill.source.installation_id"));
  assert.ok(attributes.has("chill.privacy.capture_class"));
  assert.ok(attributes.has("chill.clock.sequence_number"));
  assert.ok(attributes.has("chill.context.page.surface_id"));
  assert.ok(attributes.has("chill.context.page.path_instance_ids"));
  assert.ok(attributes.has("chill.annotation.app.area"));
  assert.ok(attributes.has("chill.privacy.annotation_classification.app.area"));
  const eventAttributes = new Map(records.at(-1)!.attributes.map(attribute => [attribute.key, attribute.value] as const));
  assert.deepEqual(eventAttributes.get("chill.record.id"), eventAttributes.get("chill.subject.id"));
});

test("browser privacy policy is default-deny before durable buffering", async () => {
  const dom = new JSDOM("", { url: "https://app.example/path?token=secret#frag" });
  Object.defineProperty(globalThis, "location", { configurable: true, value: dom.window.location });
  const storage = dom.window.localStorage;
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "privacy-v1", consent: "granted", storage, fetch: async () => new Response(null, { status: 200 }) });
  client.setContext(AnnotationContext.empty().with(annotationKey("app.area"), "checkout"));
  client.event("checkout.submit", { email: "cat@example.test", allowed: "nope" });
  const records = JSON.parse(storage.getItem("chill.buffer.v1") ?? "[]") as Array<{ source: { page_url: string }; annotations: Record<string, unknown>; payload: Record<string, unknown> }>;
  const record = records.at(-1)!;
  assert.equal(record.source.page_url, "https://app.example/path");
  assert.deepEqual(record.annotations, {});
  assert.deepEqual(record.payload, {});
});

test("consent denial aborts an in-flight browser upload and clears the queue", async () => {
  const dom = new JSDOM("", { url: "https://app.example/path" });
  const storage = dom.window.localStorage;
  let signal: AbortSignal | undefined;
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "privacy-v1", consent: "granted", storage, fetch: async (_input, init) => {
    signal = init?.signal ?? undefined;
    await new Promise((_resolve, reject) => signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")), { once: true }));
    return new Response(null, { status: 200 });
  } });
  client.event("checkout.submit");
  const flushing = client.flush();
  await new Promise(resolve => setTimeout(resolve, 0));
  client.setConsent("denied");
  assert.equal(await flushing, 0);
  assert.equal(signal?.aborted, true);
  assert.equal(storage.getItem("chill.buffer.v1"), "[]");
});

test("flushes stay below the browser keepalive body limit and drain in bounded batches", async () => {
  const dom = new JSDOM("", { url: "https://app.example/path" });
  const storage = dom.window.localStorage;
  const sizes: number[] = [];
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "privacy-v1", consent: "granted", storage, allowedPayloadKeys: ["bulk"], fetch: async (_input, init) => {
    sizes.push(new TextEncoder().encode(String(init?.body)).byteLength);
    assert.equal(init?.keepalive, true);
    return new Response(null, { status: 200 });
  } });
  for (let index = 0; index < 200; index += 1) client.event("load.sample", { bulk: "x".repeat(256) });
  let exported = 0;
  for (;;) {
    const count = await client.flush();
    if (count === 0) break;
    exported += count;
  }
  assert.equal(exported, 201);
  assert.ok(sizes.length > 1);
  assert.ok(sizes.every(size => size <= 60 * 1024));
});

test("invalid restored records cannot permanently poison the durable queue", async () => {
  const dom = new JSDOM("", { url: "https://app.example/path" });
  const storage = dom.window.localStorage;
  storage.setItem("chill.buffer.v1", JSON.stringify([{ clock: null, credential: "must-not-survive" }]));
  let body = "";
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "privacy-v1", consent: "granted", storage, fetch: async (_input, init) => {
    body = String(init?.body);
    return new Response(null, { status: 200 });
  } });
  client.event("queue.recovered");
  assert.equal(await client.flush(), 2);
  assert.doesNotMatch(body, /must-not-survive/);
  assert.equal(storage.getItem("chill.buffer.v1"), "[]");
});

test("browser fetch is invoked with the global receiver", async () => {
  let receiver: unknown;
  const fetcher = async function (this: unknown): Promise<Response> {
    receiver = this;
    return new Response(null, { status: 200 });
  } as typeof fetch;
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "v1", consent: "granted", fetch: fetcher });
  assert.equal(await client.flush(), 1);
  assert.equal(receiver, globalThis);
});

test("activity wrapper preserves return and records failure", async () => {
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "v1", consent: "granted", fetch: async () => new Response(null, { status: 200 }) });
  const add = instrumentActivity(client, "math.add", async (left: number, right: number) => left + right);
  assert.equal(await add(2, 3), 5);
});

test("traceparent round trips", () => {
  const parsed = parseTraceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"); assert.ok(parsed); assert.equal(traceparent(parsed), "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01");
  assert.equal(parseTraceparent("00-00000000000000000000000000000000-00f067aa0ba902b7-01"), undefined);
});

test("native activation observations are deduplicated by surface", () => {
  const client = new ChillBrowser({ endpoint: "https://ingest.example", sdkKey: "secret", policyVersion: "v1", consent: "granted", fetch: async () => new Response(null, { status: 200 }) });
  assert.equal(client.observeAction("cat.adopt", "touch-1", "main"), true); assert.equal(client.observeAction("cat.adopt", "touch-1", "main"), false); assert.equal(client.diagnostics.at(-1)?.code, "action.duplicate_observation");
});
