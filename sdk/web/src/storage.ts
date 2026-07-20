import type { BehaviorRecord } from "./types.js";

const STORAGE_KEY = "chill.buffer.v1";

function object(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function stringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(item => typeof item === "string");
}

function behaviorRecord(value: unknown): value is BehaviorRecord {
  const record = object(value);
  const clock = object(record?.clock);
  const source = object(record?.source);
  const context = object(record?.context);
  const page = context?.page === undefined ? undefined : object(context.page);
  const privacy = object(record?.privacy);
  return Boolean(record && clock && source && context && privacy
    && typeof record.schema_version === "string" && typeof record.schema_url === "string"
    && typeof record.record_id === "string" && typeof record.subject_id === "string"
    && typeof record.kind === "string" && typeof record.operation === "string" && typeof record.name === "string"
    && typeof clock.wall_unix_nano === "string" && typeof clock.monotonic_nano === "string" && typeof clock.sequence_number === "number"
    && typeof source.platform === "string" && typeof source.sdk_version === "string" && typeof source.installation_id === "string" && typeof source.page_url === "string"
    && typeof context.session_id === "string"
    && (!page || (typeof page.surfaceId === "string" && typeof page.instanceId === "string" && stringArray(page.path) && stringArray(page.pathInstanceIds)))
    && object(record.annotations) && object(record.annotation_classifications) && object(record.payload)
    && typeof privacy.consent === "string" && typeof privacy.policy_version === "string"
    && typeof privacy.capture_class === "string" && typeof privacy.redaction_state === "string");
}

export class DurableBuffer {
  private records: BehaviorRecord[];
  constructor(private readonly storage: Storage | undefined, private readonly maximum: number) {
    try {
      const value = storage?.getItem(STORAGE_KEY);
      const parsed: unknown = value ? JSON.parse(value) : [];
      this.records = Array.isArray(parsed) ? parsed.filter(behaviorRecord) : [];
      if (value && this.records.length !== (Array.isArray(parsed) ? parsed.length : 0)) this.persist();
    }
    catch { this.records = []; }
  }
  push(record: BehaviorRecord): void { this.records.push(record); if (this.records.length > this.maximum) this.records.splice(0, this.records.length - this.maximum); this.persist(); }
  peek(count: number): readonly BehaviorRecord[] { return this.records.slice(0, count); }
  remove(count: number): void { this.records.splice(0, count); this.persist(); }
  clear(): void { this.records = []; this.persist(); }
  get length(): number { return this.records.length; }
  private persist(): void { try { this.storage?.setItem(STORAGE_KEY, JSON.stringify(this.records)); } catch { /* Memory remains bounded when storage is unavailable. */ } }
}
