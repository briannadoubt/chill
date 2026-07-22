import assert from "node:assert/strict";
import test from "node:test";

import { createRuntime, isTraceparent } from "../dist/index.js";

const attrs = (log) =>
  Object.fromEntries(
    log.attributes.map((attribute) => [
      attribute.key,
      Object.values(attribute.value)[0],
    ]),
  );

test("OTLP contains strict Chill V1 attributes and bearer auth", async () => {
  const sent = [];
  const runtime = createRuntime({
    serviceName: "api",
    endpoint: "https://collector.test/v1/logs",
    sdkKey: "key-memory-only",
    installationId: "9f22f5c1-c67f-4c2d-9f04-0b1fc3000001",
    policyVersion: "portable-v1",
    annotationKeys: ["http.method"],
    fetch: async (_url, init) => {
      sent.push([init.headers, JSON.parse(init.body)]);
      return new Response("", { status: 200 });
    },
  });
  await runtime.setConsent("granted");
  await runtime.record({
    name: "user.login",
    annotations: { "http.method": "POST", secret: "never" },
  });
  await runtime.record({ name: "user.submit", kind: "action" });
  await runtime.flush();

  const [headers, body] = sent[0];
  assert.equal(
    new Headers(headers).get("authorization"),
    "Bearer key-memory-only",
  );
  const scope = body.resourceLogs[0].scopeLogs[0];
  assert.equal(
    scope.schemaUrl,
    "https://schemas.chill.dev/behavior/v1/envelope.schema.json",
  );
  const log = scope.logRecords[0];
  const attributes = attrs(log);
  for (const key of [
    "chill.schema.version",
    "chill.schema.url",
    "chill.record.id",
    "chill.subject.id",
    "chill.behavior.kind",
    "chill.behavior.operation",
    "chill.behavior.name",
    "chill.clock.sequence_number",
    "chill.source.platform",
    "chill.source.installation_id",
    "chill.source.process_id",
    "chill.context.session_id",
    "chill.privacy.consent",
    "chill.privacy.policy_version",
    "chill.privacy.capture_class",
    "chill.privacy.redaction_state",
  ]) {
    assert.ok(attributes[key]);
  }
  assert.equal(attributes["chill.behavior.kind"], "event");
  assert.equal(attributes["chill.behavior.operation"], "instant");
  assert.equal(attributes["chill.record.id"], attributes["chill.subject.id"]);
  assert.equal(attributes["chill.source.platform"], "server");
  assert.equal(attributes["chill.payload.event_class"], "custom");
  assert.equal(attributes["chill.payload.emission"], "observed");
  assert.equal(attributes["chill.annotation.http.method"], "POST");
  assert.equal(attributes["chill.annotation.secret"], undefined);
  assert.equal(
    typeof log.attributes.find(
      (attribute) => attribute.key === "chill.clock.sequence_number",
    ).value.intValue,
    "string",
  );
  assert.match(log.timeUnixNano, /^\d+$/);
  assert.match(log.traceId, /^[0-9a-f]{32}$/);
  assert.match(log.spanId, /^[0-9a-f]{16}$/);
  assert.doesNotMatch(
    JSON.stringify(body),
    /key-memory-only|secret|never|authorization|\/v1\/logs/,
  );
  const action = attrs(scope.logRecords[1]);
  assert.equal(action["chill.behavior.kind"], "action");
  assert.equal(action["chill.payload.element_id"], "user.submit");
  assert.equal(action["chill.payload.role"], "operation");
  assert.equal(action["chill.payload.activation"], "system");
  assert.equal(action["chill.payload.input"], "system");
});

test("retry preserves record identity and default deny", async () => {
  const bodies = [];
  let failures = 1;
  const runtime = createRuntime({
    serviceName: "api",
    endpoint: "https://x.test/v1/logs",
    sdkKey: "k",
    fetch: async (_url, init) => {
      bodies.push(JSON.parse(init.body));
      return new Response("", { status: failures-- ? 500 : 200 });
    },
  });
  assert.equal(await runtime.record({ name: "x.y" }), false);
  await runtime.setConsent("granted");
  await runtime.record({ name: "x.y" });
  assert.equal(await runtime.flush(), false);
  assert.equal(await runtime.flush(), true);
  const first = attrs(
    bodies[0].resourceLogs[0].scopeLogs[0].logRecords[0],
  );
  const retry = attrs(
    bodies[1].resourceLogs[0].scopeLogs[0].logRecords[0],
  );
  assert.equal(first["chill.record.id"], retry["chill.record.id"]);
});

test("activity pairs schema-valid start/end identity and trace", async () => {
  const sent = [];
  const runtime = createRuntime({
    serviceName: "api",
    endpoint: "https://x.test/v1/logs",
    sdkKey: "k",
    fetch: async (_url, init) => {
      sent.push(JSON.parse(init.body));
      return new Response();
    },
  });
  await runtime.setConsent("granted");
  const traces = await Promise.all(
    [1, 2, 3].map(() =>
      runtime.activity("task.work", async () => {
        await Promise.resolve();
        return runtime.current().traceparent;
      }),
    ),
  );
  assert.equal(new Set(traces).size, 3);
  for (const trace of traces) assert.ok(isTraceparent(trace));
  await assert.rejects(
    runtime.activity("task.fail", () => {
      throw new Error("private");
    }),
  );
  await runtime.flush();

  const paired = sent[0].resourceLogs[0].scopeLogs[0].logRecords.filter(
    (log) => log.eventName === "task.work",
  );
  const groups = new Map();
  for (const log of paired) {
    const attributes = attrs(log);
    assert.equal(attributes["chill.behavior.kind"], "activity");
    assert.equal(attributes["chill.payload.activity_kind"], "custom");
    assert.equal(attributes["chill.payload.role"], "operation");
    assert.equal(attributes["chill.payload.attempt"], "1");
    assert.equal(attributes["chill.payload.recursion_depth"], "0");
    const subject = attributes["chill.subject.id"];
    groups.set(subject, [...(groups.get(subject) ?? []), log]);
  }
  assert.equal(groups.size, 3);
  for (const logs of groups.values()) {
    assert.equal(logs.length, 2);
    const start = attrs(logs[0]);
    const end = attrs(logs[1]);
    assert.deepEqual(
      new Set([
        start["chill.behavior.operation"],
        end["chill.behavior.operation"],
      ]),
      new Set(["start", "end"]),
    );
    assert.equal(logs[0].traceId, logs[1].traceId);
    const duration = logs[1].attributes.find(
      (attribute) => attribute.key === "chill.duration_nano",
    );
    assert.equal(typeof duration.value.stringValue, "string");
  }
  assert.doesNotMatch(JSON.stringify(sent), /private|stack/);
});

test("lifecycle is paired and idempotent", async () => {
  const sent = [];
  const runtime = createRuntime({
    serviceName: "api",
    endpoint: "https://x.test/v1/logs",
    sdkKey: "k",
    fetch: async (_url, init) => {
      sent.push(JSON.parse(init.body));
      return new Response();
    },
  });
  await runtime.setConsent("granted");
  await runtime.start();
  await runtime.start();
  await runtime.stop();
  await runtime.stop();
  const logs = sent[0].resourceLogs[0].scopeLogs[0].logRecords;
  assert.equal(logs.length, 2);
  const first = attrs(logs[0]);
  const second = attrs(logs[1]);
  assert.equal(first["chill.behavior.kind"], "session");
  assert.equal(second["chill.behavior.kind"], "session");
  assert.equal(first["chill.subject.id"], second["chill.subject.id"]);
  assert.deepEqual(
    [first["chill.behavior.operation"], second["chill.behavior.operation"]],
    ["start", "end"],
  );
});

test("invalid values, endpoints, and untrusted propagation are rejected", async () => {
  for (const endpoint of [
    "https://x.test/not-logs",
    "http://x.test/v1/logs",
    "https://x.test/v1/logs?secret=no",
  ]) {
    assert.throws(() =>
      createRuntime({ serviceName: "x", endpoint, sdkKey: "k" }),
    );
  }
  assert.doesNotThrow(() =>
    createRuntime({
      serviceName: "x",
      endpoint: "http://127.0.0.1:4318/v1/logs",
      sdkKey: "k",
    }),
  );
  assert.throws(() =>
    createRuntime({ serviceName: "x", annotationKeys: ["bad-key"] }),
  );
  const calls = [];
  const runtime = createRuntime({
    serviceName: "x",
    trustedOrigins: ["https://good.test"],
    fetch: async (url, init) => {
      calls.push([String(url), new Headers(init.headers).get("traceparent")]);
      return new Response();
    },
  });
  await assert.rejects(runtime.record({ name: "Bad" }));
  await runtime.setConsent("granted");
  await assert.rejects(
    runtime.record({ name: "x.y", annotations: { a: Infinity } }),
  );
  await runtime.activity("x.y", async () => {
    await runtime.fetch("https://good.test/private?q=no");
    await runtime.fetch("https://bad.test/private?q=no");
  });
  assert.ok(isTraceparent(calls[0][1]));
  assert.equal(calls[1][1], null);
});
