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

test("analytics editors use scoped patch routes and preserve exact wire fields", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  const requests: Array<{ input: string; method?: string; body: unknown }> = [];
  globalThis.fetch = async (input, init) => {
    requests.push({
      input: String(input),
      method: init?.method,
      body: JSON.parse(String(init?.body)),
    });
    return Response.json({
      id: "resource-id",
      name: "Product pulse",
      description: "",
      plan: {},
      visualization: "line",
      sharing: "organization",
      layout: [],
      updated_at: "2026-07-24T12:00:00Z",
    });
  };
  const api = new ChillApi("", "ch_us_test-session-secret");
  await api.updateSavedQuery("query/id", {
    name: "Product pulse",
    description: "",
    plan: { version: 1 },
    visualization: "line",
  });
  await api.updateDashboard("dashboard/id", {
    name: "Product pulse",
    description: "",
    sharing: "organization",
    layout: [{ saved_query_id: "query-id", width: 2 }],
  });
  assert.deepEqual(requests, [
    {
      input: "/v1/console/saved-queries/query%2Fid",
      method: "PATCH",
      body: {
        name: "Product pulse",
        description: "",
        plan: { version: 1 },
        visualization: "line",
      },
    },
    {
      input: "/v1/console/dashboards/dashboard%2Fid",
      method: "PATCH",
      body: {
        name: "Product pulse",
        description: "",
        sharing: "organization",
        layout: [{ saved_query_id: "query-id", width: 2 }],
      },
    },
  ]);
});

test("query requests are serialized for the single-engine Rust service", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  const order: string[] = [];
  let releaseFirst: (() => void) | undefined;
  const firstPending = new Promise<void>((resolve) => { releaseFirst = resolve; });
  globalThis.fetch = async (_input, init) => {
    const plan = JSON.parse(String(init?.body)).plan as { kind: string };
    order.push(`start:${plan.kind}`);
    if (plan.kind === "events") await firstPending;
    order.push(`finish:${plan.kind}`);
    return Response.json({
      columns: [],
      rows: [],
      stats: { cache_hit: false, file_count: 0, scan_bytes: 0, row_count: 0, total_duration_nano: 0 },
    });
  };
  const api = new ChillApi("", "ch_us_test-session-secret");
  const first = api.query("project", "environment", { kind: "events" });
  const second = api.query("project", "environment", { kind: "aggregate" });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(order, ["start:events"]);
  releaseFirst?.();
  await Promise.all([first, second]);
  assert.deepEqual(order, [
    "start:events",
    "finish:events",
    "start:aggregate",
    "finish:aggregate",
  ]);
});
