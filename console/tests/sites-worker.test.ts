import assert from "node:assert/strict";
import test from "node:test";
import worker from "../sites-worker.js";

test("Sites worker exchanges only the platform-authenticated email", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  globalThis.fetch = async (input, init) => {
    assert.equal(input, "https://chill-api.example.test/v1/auth/sites-session");
    assert.equal(init?.method, "POST");
    assert.deepEqual(JSON.parse(String(init?.body)), { email: "owner@example.test" });
    const headers = init?.headers as Record<string, string>;
    assert.equal(headers["x-chill-sites-auth"], "server-only-secret");
    return Response.json({
      credential: "ch_us_TEST_ONLY_NOT_A_CREDENTIAL",
      expiresAt: "2026-07-19T12:00:00Z",
    }, { status: 201 });
  };

  const response = await worker.fetch(new Request("https://console.example.test/api/session", {
    method: "POST",
    headers: { "oai-authenticated-user-email": "owner@example.test" },
  }), {
    CHILL_API_ORIGIN: "https://chill-api.example.test/",
    CHILL_SITES_AUTH_SECRET: "server-only-secret",
  });
  assert.equal(response.status, 201);
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.deepEqual(await response.json(), {
    apiBase: "https://chill-api.example.test",
    credential: "ch_us_TEST_ONLY_NOT_A_CREDENTIAL",
    expiresAt: "2026-07-19T12:00:00Z",
  });
});

test("Sites worker rejects requests without platform identity", async () => {
  const response = await worker.fetch(
    new Request("https://console.example.test/api/session", { method: "POST" }),
    {},
  );
  assert.equal(response.status, 401);
});

test("Sites worker relays authenticated telemetry with only its server-side SDK key", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  const payload = { resourceLogs: [{ scopeLogs: [{ logRecords: [] }] }] };
  globalThis.fetch = async (input, init) => {
    assert.equal(input, "https://chill-api.example.test/v1/logs");
    assert.equal(init?.method, "POST");
    assert.equal(init?.body, JSON.stringify(payload));
    const headers = init?.headers as Record<string, string>;
    assert.equal(headers.authorization, "Bearer sdk_server_only");
    assert.equal(headers["x-chill-schema-version"], "1.0.0");
    assert.equal(headers["oai-authenticated-user-email"], undefined);
    return new Response(null, { status: 202 });
  };

  const response = await worker.fetch(new Request("https://console.example.test/v1/logs", {
    method: "POST",
    headers: {
      "authorization": "Bearer sites-authenticated-relay",
      "content-type": "application/json",
      "oai-authenticated-user-email": "owner@example.test",
    },
    body: JSON.stringify(payload),
  }), {
    CHILL_API_ORIGIN: "https://chill-api.example.test/",
    CHILL_CONSOLE_SDK_KEY: "sdk_server_only",
  });

  assert.equal(response.status, 202);
  assert.equal(response.headers.get("cache-control"), "no-store");
});

test("Sites worker refuses unauthenticated or malformed telemetry", async () => {
  const unauthenticated = await worker.fetch(new Request("https://console.example.test/v1/logs", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ resourceLogs: [{}] }),
  }), {});
  assert.equal(unauthenticated.status, 401);

  const malformed = await worker.fetch(new Request("https://console.example.test/v1/logs", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "oai-authenticated-user-email": "owner@example.test",
    },
    body: JSON.stringify({ resourceLogs: [] }),
  }), {
    CHILL_API_ORIGIN: "https://chill-api.example.test",
    CHILL_CONSOLE_SDK_KEY: "sdk_server_only",
  });
  assert.equal(malformed.status, 400);
});
