import assert from "node:assert/strict";
import test from "node:test";
import {
  controlActionName,
  createConsoleTelemetry,
  telemetryFailureCode,
} from "../src/telemetry.ts";

test("console telemetry uses the authenticated relay and excludes application secrets", async () => {
  let authorization = "";
  let body = "";
  const client = createConsoleTelemetry({
    endpoint: "https://console.example.test",
    policyVersion: "privacy-v1",
    fetch: async (_input, init) => {
      authorization = new Headers(init?.headers).get("authorization") ?? "";
      body = String(init?.body);
      return new Response(null, { status: 202 });
    },
  });

  client.startPage("overview");
  client.event("console.navigate.explore", { event_class: "lifecycle", emission: "observed" });
  assert.equal(await client.flush(), 3);
  assert.equal(authorization, "Bearer sites-authenticated-relay");
  assert.match(body, /chill\.source\.platform/);
  assert.match(body, /console\.navigate\.explore/);
  assert.doesNotMatch(body, /ch_us_|sdk_live_|owner@example/);
});

test("control-plane labels map to stable semantic action names", () => {
  assert.equal(controlActionName("Creating data source"), "console.data_source.create");
  assert.equal(controlActionName("Rotating SDK key"), "console.sdk_key.rotate");
  assert.equal(controlActionName("Unrecognized future action"), "console.control.change");
});

test("telemetry failures expose only bounded content-free diagnostic codes", () => {
  assert.equal(
    telemetryFailureCode(new Error("Chill export failed with HTTP 502")),
    "export_http_error",
  );
  assert.equal(telemetryFailureCode(new Error("Chill client failed during encoding")), "client_encode_error");
  assert.equal(telemetryFailureCode(new Error("Chill client failed during endpoint resolution")), "client_endpoint_error");
  assert.equal(telemetryFailureCode(new Error("Chill client failed during transport")), "client_transport_error");
  assert.equal(
    telemetryFailureCode(new Error("credential ch_sk_must-never-appear")),
    "client_error",
  );
});
