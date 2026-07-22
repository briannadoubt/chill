import type { AnnotationPrimitive, BehaviorRecord } from "./types.js";

const MAXIMUM_KEEPALIVE_BODY_BYTES = 60 * 1024;

function otlpValue(value: AnnotationPrimitive): Record<string, unknown> {
  if (typeof value === "string") return { stringValue: value };
  if (typeof value === "boolean") return { boolValue: value };
  if (typeof value === "number") return Number.isInteger(value) ? { intValue: `${value}` } : { doubleValue: value };
  return { arrayValue: { values: value.map(item => ({ stringValue: item })) } };
}

function flatten(record: BehaviorRecord): Array<{ key: string; value: Record<string, unknown> }> {
  const attributes: Record<string, AnnotationPrimitive> = {
    "chill.record.id": record.record_id, "chill.subject.id": record.subject_id, "chill.behavior.kind": record.kind,
    "chill.behavior.operation": record.operation, "chill.behavior.name": record.name, "chill.context.session_id": record.context.session_id,
    "chill.schema.version": record.schema_version, "chill.schema.url": record.schema_url, "chill.privacy.consent": record.privacy.consent,
    "chill.privacy.policy_version": record.privacy.policy_version, "chill.privacy.capture_class": record.privacy.capture_class,
    "chill.privacy.redaction_state": record.privacy.redaction_state, "chill.clock.sequence_number": record.clock.sequence_number,
    "chill.source.platform": record.source.platform, "chill.source.installation_id": record.source.installation_id,
  };
  if (record.context.replay_id) attributes["chill.context.replay_id"] = record.context.replay_id;
  for (const [key, value] of Object.entries(record.annotations)) {
    attributes[`chill.annotation.${key}`] = value;
    attributes[`chill.privacy.annotation_classification.${key}`] = record.annotation_classifications[key] ?? "internal";
  }
  if (record.context.page) {
    attributes["chill.context.page.surface_id"] = record.context.page.surfaceId;
    attributes["chill.context.page.instance_id"] = record.context.page.instanceId;
    attributes["chill.context.page.path"] = record.context.page.path;
    attributes["chill.context.page.path_instance_ids"] = record.context.page.pathInstanceIds;
  }
  for (const [key, value] of Object.entries(record.payload)) if (typeof value === "string" || typeof value === "number" || typeof value === "boolean" || (Array.isArray(value) && value.every(item => typeof item === "string"))) attributes[`chill.payload.${key}`] = value as AnnotationPrimitive;
  return Object.entries(attributes).map(([key, value]) => ({ key, value: otlpValue(value) }));
}

function encode(records: readonly BehaviorRecord[]): string {
  return JSON.stringify({ resourceLogs: [{ resource: { attributes: [{ key: "service.name", value: { stringValue: "browser" } }, { key: "os.type", value: { stringValue: "web" } }] }, scopeLogs: [{ scope: { name: "dev.chill.web", version: "0.1.0" }, logRecords: records.map(record => ({ timeUnixNano: record.clock.wall_unix_nano, observedTimeUnixNano: record.clock.wall_unix_nano, eventName: record.name, traceId: record.trace?.traceId, spanId: record.trace?.spanId, flags: record.trace?.sampled ? 1 : 0, attributes: flatten(record) })) }] }] });
}

function boundedBody(records: readonly BehaviorRecord[]): { body: string; count: number; keepalive: boolean } {
  let low = 1;
  let high = records.length;
  let selectedBody = "";
  let selectedCount = 0;
  while (low <= high) {
    const count = Math.floor((low + high) / 2);
    const body = encode(records.slice(0, count));
    if (new TextEncoder().encode(body).byteLength <= MAXIMUM_KEEPALIVE_BODY_BYTES) {
      selectedBody = body;
      selectedCount = count;
      low = count + 1;
    } else {
      high = count - 1;
    }
  }
  if (selectedCount > 0) return { body: selectedBody, count: selectedCount, keepalive: true };
  return { body: encode(records.slice(0, 1)), count: 1, keepalive: false };
}

export class OtlpExporter {
  constructor(private readonly endpoint: string, private readonly sdkKey: string, private readonly fetcher: typeof fetch = fetch) {}
  async export(records: readonly BehaviorRecord[], signal?: AbortSignal): Promise<number> {
    let batch: ReturnType<typeof boundedBody>;
    try { batch = boundedBody(records); }
    catch { throw new Error("Chill client failed during encoding"); }
    const init: RequestInit = { method: "POST", headers: { "authorization": `Bearer ${this.sdkKey}`, "content-type": "application/json", "x-chill-schema-version": "1.0.0" }, body: batch.body, keepalive: batch.keepalive };
    if (signal) init.signal = signal;
    let target: URL;
    try { target = new URL("/v1/logs", this.endpoint); }
    catch { throw new Error("Chill client failed during endpoint resolution"); }
    let response: Response;
    try { response = await this.fetcher.call(globalThis, target, init); }
    catch { throw new Error("Chill client failed during transport"); }
    if (!response.ok) throw new Error(`Chill export failed with HTTP ${response.status}`);
    return batch.count;
  }
}
