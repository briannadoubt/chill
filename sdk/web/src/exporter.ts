import type { AnnotationPrimitive, BehaviorRecord } from "./types.js";

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

export class OtlpExporter {
  constructor(private readonly endpoint: string, private readonly sdkKey: string, private readonly fetcher: typeof fetch = fetch) {}
  async export(records: readonly BehaviorRecord[]): Promise<void> {
    const payload = { resourceLogs: [{ resource: { attributes: [{ key: "service.name", value: { stringValue: "browser" } }, { key: "os.type", value: { stringValue: "web" } }] }, scopeLogs: [{ scope: { name: "dev.chill.web", version: "0.1.0" }, logRecords: records.map(record => ({ timeUnixNano: record.clock.wall_unix_nano, observedTimeUnixNano: record.clock.wall_unix_nano, eventName: record.name, traceId: record.trace?.traceId, spanId: record.trace?.spanId, flags: record.trace?.sampled ? 1 : 0, attributes: flatten(record) })) }] }] };
    const response = await this.fetcher(new URL("/v1/logs", this.endpoint), { method: "POST", headers: { "authorization": `Bearer ${this.sdkKey}`, "content-type": "application/json", "x-chill-schema-version": "1.0.0" }, body: JSON.stringify(payload), keepalive: true });
    if (!response.ok) throw new Error(`Chill export failed with HTTP ${response.status}`);
  }
}
