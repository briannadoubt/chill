import assert from "node:assert/strict";
import test from "node:test";
import { ChillApi } from "../src/api.ts";

test("overview sends the user session only in the authorization header", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  globalThis.fetch = async (input, init) => {
    assert.equal(input, "https://api.example.test/v1/console/overview");
    const headers = init?.headers as Record<string, string>;
    assert.equal(headers.authorization, "Bearer ch_us_test-session-secret");
    assert.equal(init?.cache, "no-store");
    assert.equal(init?.body, undefined);
    return Response.json({ actor: {}, organization: {}, projects: [] });
  };

  const result = await new ChillApi("https://api.example.test", "ch_us_test-session-secret").overview();
  assert.deepEqual(result.projects, []);
});

test("mutations use exact Rust wire names and accept empty responses", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  globalThis.fetch = async (input, init) => {
    assert.equal(input, "/v1/console/environments/env-id/retention");
    assert.equal(init?.method, "PATCH");
    assert.deepEqual(JSON.parse(String(init?.body)), { retention_days: 90 });
    return new Response(null, { status: 204 });
  };

  await new ChillApi("", "ch_us_test-session-secret").updateRetention("env-id", 90);
});

test("one-time credential responses remain available to the caller", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  globalThis.fetch = async () => Response.json({ key_id: "key-id", credential: "ch_sk_once-only" }, { status: 201 });

  const issued = await new ChillApi("", "ch_us_test-session-secret").rotateSdkKey("key-id");
  assert.equal(issued.credential, "ch_sk_once-only");
});

test("authorization failures produce a session-expiry error", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  globalThis.fetch = async () => new Response("unauthorized", { status: 401 });

  await assert.rejects(
    new ChillApi("", "ch_us_expired-session").overview(),
    /session is invalid or expired/,
  );
});
