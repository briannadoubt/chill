import assert from "node:assert/strict";
import test from "node:test";
import { commandOptions, eventEnvelope, invokeWithCarrier, parseCarrier } from "../src/index.js";

const carrier = { version: 1 as const, traceparent: "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01", semanticName: "account.save" };
test("command carrier is bounded metadata in invoke headers", () => {
  const options = commandOptions(carrier);
  assert.deepEqual(parseCarrier(options.headers!["x-chill-carrier"]!), carrier);
});
test("event carrier is explicit and does not transform payload", () => {
  const payload = { private: "do-not-read" };
  assert.strictEqual(eventEnvelope(carrier, payload).payload, payload);
});
test("invoke preserves opaque application arguments", async () => {
  const args = { private: "do-not-read" };
  const invoke = async <T>(command: string, received?: unknown, options?: { readonly headers?: Readonly<Record<string, string>> }): Promise<T> => {
    assert.equal(command, "save_account");
    assert.strictEqual(received, args);
    assert.deepEqual(parseCarrier(options?.headers?.["x-chill-carrier"] ?? ""), carrier);
    return undefined as T;
  };
  await invokeWithCarrier(invoke, "save_account", args, carrier);
});
test("invalid carrier is rejected before headers are created", () => {
  assert.throws(() => commandOptions({ ...carrier, semanticName: "UPPER" }));
  assert.throws(() => commandOptions({ ...carrier, semanticName: "bad..name" }));
  assert.throws(() => commandOptions({ ...carrier, traceparent: "00-0123456789ABCDEF0123456789ABCDEF-0123456789abcdef-01" }));
});
test("baggage defaults to deny and requires an explicit allowlist", () => {
  const withBaggage = { ...carrier, baggage: { "public.region": "us" } };
  assert.throws(() => commandOptions(withBaggage));
  const header = commandOptions(withBaggage, { baggageAllowlist: ["public.region"] }).headers!["x-chill-carrier"]!;
  assert.deepEqual(parseCarrier(header, { baggageAllowlist: ["public.region"] }), withBaggage);
  assert.equal(parseCarrier(header), undefined);
});
test("privacy canaries are never serialized, inspected, or transformed", async () => {
  const args = new Proxy({ secret: "command-args" }, { ownKeys: () => { throw new Error("args enumerated"); } });
  const payload = new Proxy({ secret: "event-payload" }, { ownKeys: () => { throw new Error("payload enumerated"); } });
  const result = new Proxy({ secret: "command-result" }, { ownKeys: () => { throw new Error("result enumerated"); } });
  const envelope = eventEnvelope(carrier, payload);
  assert.strictEqual(envelope.payload, payload);
  const output = await invokeWithCarrier(async <T>(_command: string, received?: unknown) => { assert.strictEqual(received, args); return result as T; }, "save_account", args, carrier);
  assert.strictEqual(output, result);
});
test("strict trace flags reject arbitrary values", () => assert.throws(() => commandOptions({ ...carrier, traceparent: "00-0123456789abcdef0123456789abcdef-0123456789abcdef-02" })));
