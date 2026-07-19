import assert from "node:assert/strict";
import test from "node:test";
import {
  apiUrl,
  bootstrapPrivateSession,
  credentialPreview,
  errorMessageForStatus,
  normalizeApiBase,
  parseJsonObject,
  samplingPercentage,
} from "../src/api-contract.ts";

test("private Sites login exchanges identity without exposing a server secret", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  globalThis.fetch = async (input, init) => {
    assert.equal(input, "/api/session");
    assert.equal(init?.method, "POST");
    assert.equal(init?.cache, "no-store");
    assert.deepEqual(init?.headers, { accept: "application/json" });
    return Response.json({
      apiBase: "https://api.example.test/",
      credential: "ch_us_TEST_ONLY_NOT_A_CREDENTIAL",
      expiresAt: "2026-07-19T12:00:00Z",
    }, { status: 201 });
  };

  const issued = await bootstrapPrivateSession();
  assert.equal(issued.apiBase, "https://api.example.test");
  assert.match(issued.credential, /^ch_us_/);
});

test("private Sites login reports unauthorized workspace identities", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => { globalThis.fetch = originalFetch; });
  globalThis.fetch = async () => new Response("unauthorized", { status: 401 });
  await assert.rejects(bootstrapPrivateSession(), /not authorized/);
});

test("normalizes same-origin and absolute Rust API addresses", () => {
  assert.equal(normalizeApiBase(""), "");
  assert.equal(normalizeApiBase(" /control/ "), "/control");
  assert.equal(normalizeApiBase("https://chill.example.test///"), "https://chill.example.test");
  assert.equal(apiUrl("https://chill.example.test/", "/v1/console/overview"), "https://chill.example.test/v1/console/overview");
});

test("rejects unsafe or ambiguous API addresses", () => {
  assert.throws(() => normalizeApiBase("ftp://example.test"), /http/);
  assert.throws(() => normalizeApiBase("https://user:secret@example.test"), /credentials/);
  assert.throws(() => normalizeApiBase("https://example.test/?token=nope"), /query/);
  assert.throws(() => apiUrl("", "v1/console/overview"), /start with a slash/);
});

test("privacy and schema inputs must be JSON objects", () => {
  assert.deepEqual(parseJsonObject('{"type":"object"}', "Schema"), { type: "object" });
  assert.throws(() => parseJsonObject("[1,2]", "Policy"), /JSON object/);
  assert.throws(() => parseJsonObject("not-json", "Policy"), /valid JSON/);
});

test("renders exact sampling rates without dividing by invalid denominators", () => {
  assert.equal(samplingPercentage(1, 4), "25%");
  assert.equal(samplingPercentage(1, 3), "33.33%");
  assert.equal(samplingPercentage(1, 0), "—");
});

test("maps security-sensitive API failures to actionable messages", () => {
  assert.match(errorMessageForStatus(401), /expired/);
  assert.match(errorMessageForStatus(403), /role/);
  assert.match(errorMessageForStatus(409), /conflicts/);
  assert.match(errorMessageForStatus(422), /rejected/);
  assert.match(errorMessageForStatus(503), /could not complete/);
  assert.equal(errorMessageForStatus(418, "teapot"), "teapot");
});

test("credential previews preserve enough material for visual verification", () => {
  assert.equal(credentialPreview("short"), "short");
  assert.equal(
    credentialPreview("ch_sk_TEST_ONLY_NOT_A_CREDENTIAL"),
    "ch_sk_TEST_O…NTIAL",
  );
});
