import { AnnotationContext, type BehaviorKind, type BehaviorRecord, type Consent, type Operation, type PageIdentity, type RuntimeConfiguration, type TraceContext } from "./types.js";
import { DurableBuffer } from "./storage.js";
import { OtlpExporter } from "./exporter.js";

const SEMANTIC_NAME = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/;
const hex = (bytes: Uint8Array) => [...bytes].map(value => value.toString(16).padStart(2, "0")).join("");

export function randomHex(bytes: number): string {
  const data = new Uint8Array(bytes); crypto.getRandomValues(data); return hex(data);
}

export function uuidV7(now = Date.now()): string {
  const bytes = new Uint8Array(16); crypto.getRandomValues(bytes);
  let timestamp = now;
  for (let index = 5; index >= 0; index -= 1) { bytes[index] = timestamp & 0xff; timestamp = Math.floor(timestamp / 256); }
  bytes[6] = (bytes[6]! & 0x0f) | 0x70; bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const value = hex(bytes); return `${value.slice(0, 8)}-${value.slice(8, 12)}-${value.slice(12, 16)}-${value.slice(16, 20)}-${value.slice(20)}`;
}

export function uuidV4(): string {
  const bytes = new Uint8Array(16); crypto.getRandomValues(bytes);
  bytes[6] = (bytes[6]! & 0x0f) | 0x40; bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const value = hex(bytes); return `${value.slice(0, 8)}-${value.slice(8, 12)}-${value.slice(12, 16)}-${value.slice(16, 20)}-${value.slice(20)}`;
}

const INSTALLATION_STORAGE_KEY = "chill.installation.v1";
const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
function installationId(storage: Storage | undefined, configured: string | undefined): string {
  if (configured) {
    if (!UUID_V4.test(configured)) throw new TypeError("installationId must be a lowercase UUIDv4");
    return configured;
  }
  try {
    const stored = storage?.getItem(INSTALLATION_STORAGE_KEY);
    if (stored && UUID_V4.test(stored)) return stored;
  } catch { /* Generate a process-local identity when storage is unavailable. */ }
  const generated = uuidV4();
  try { storage?.setItem(INSTALLATION_STORAGE_KEY, generated); } catch { /* Optional persistence. */ }
  return generated;
}

export class ChillBrowser {
  readonly installationId: string;
  readonly sessionId: string;
  readonly replayId: string;
  readonly diagnostics: Array<{ code: string; message: string; at: number }> = [];
  private readonly config: Required<Pick<RuntimeConfiguration, "policyVersion" | "sampleRate" | "replaySampleRate" | "maxBufferedRecords">> & RuntimeConfiguration;
  private readonly buffer: DurableBuffer;
  private readonly exporter: OtlpExporter;
  private sequence = 0;
  private consent: Consent;
  private enabled: boolean;
  private readonly replayEnabled: boolean;
  private readonly activations = new Map<string, number>();
  private page?: PageIdentity;
  private context = AnnotationContext.empty();
  private currentTrace: TraceContext | undefined;

  constructor(configuration: RuntimeConfiguration) {
    if (!configuration.endpoint || !configuration.sdkKey) throw new TypeError("endpoint and sdkKey are required");
    const sampleRate = configuration.sampleRate ?? 1; const replaySampleRate = configuration.replaySampleRate ?? 0;
    if (sampleRate < 0 || sampleRate > 1 || replaySampleRate < 0 || replaySampleRate > 1) throw new RangeError("sample rates must be between 0 and 1");
    this.config = { ...configuration, sampleRate, replaySampleRate, maxBufferedRecords: configuration.maxBufferedRecords ?? 2_000, policyVersion: configuration.policyVersion };
    this.consent = configuration.consent ?? "unknown";
    this.enabled = (configuration.random ?? Math.random)() < sampleRate;
    this.replayEnabled = (configuration.random ?? Math.random)() < replaySampleRate;
    this.sessionId = uuidV7(); this.replayId = uuidV7();
    this.installationId = installationId(configuration.storage, configuration.installationId);
    this.buffer = new DurableBuffer(configuration.storage, this.config.maxBufferedRecords);
    this.exporter = new OtlpExporter(configuration.endpoint, configuration.sdkKey, configuration.fetch);
    if (this.enabled && this.consent === "granted") this.emit("session", "start", "app.session", {});
  }

  setConsent(consent: Consent): void {
    const previous = this.consent; this.consent = consent;
    if (consent === "denied") this.buffer.clear();
    if (previous !== "granted" && consent === "granted" && this.enabled) this.emit("session", "start", "app.session", {});
  }
  setEnabled(enabled: boolean): void { this.enabled = enabled; }
  setContext(context: AnnotationContext): void { this.context = context; this.captureContextDiagnostics(context); }
  setTrace(trace: TraceContext | undefined): void { this.currentTrace = trace; }
  getTrace(): TraceContext | undefined { return this.currentTrace; }
  getPage(): PageIdentity | undefined { return this.page; }
  shouldReplay(): boolean { return this.enabled && this.consent === "granted" && this.replayEnabled; }

  startPage(segment: string, parent?: PageIdentity): PageIdentity {
    if (!SEMANTIC_NAME.test(segment)) throw new TypeError(`invalid page segment: ${segment}`);
    const previous = this.page;
    if (previous) this.emit("page", "end", previous.path.at(-1) ?? "page", pagePayload(previous, "end", "navigate"), previous);
    const instanceId = uuidV7();
    this.page = { surfaceId: parent?.surfaceId ?? instanceId, instanceId, path: [...(parent?.path ?? []), segment], pathInstanceIds: [...(parent?.pathInstanceIds ?? []), instanceId] };
    this.emit("page", "start", segment, pagePayload(this.page, "start", previous ? "navigate" : "initial"), this.page); return this.page;
  }

  action(name: string, payload: Readonly<Record<string, unknown>> = {}): void { this.emit("action", "instant", name, payload); }
  observeAction(name: string, nativeActivationId: string, surfaceId: string, payload: Readonly<Record<string, unknown>> = {}): boolean {
    const key = `${surfaceId}\u0000${nativeActivationId}`;
    if (this.activations.has(key)) { this.diagnostic("action.duplicate_observation", `Suppressed duplicate activation ${nativeActivationId}`); return false; }
    this.activations.set(key, Date.now());
    if (this.activations.size > 512) this.activations.delete(this.activations.keys().next().value!);
    this.action(name, { ...payload, native_activation_id: nativeActivationId, surface_id: surfaceId }); return true;
  }
  event(name: string, payload: Readonly<Record<string, unknown>> = {}): void { this.emit("event", "instant", name, payload); }
  impression(name: string, payload: Readonly<Record<string, unknown>> = {}): void { this.emit("impression", "instant", name, payload); }
  replay(payload: Readonly<Record<string, unknown>>): void { this.emit("replay", "instant", "session.replay", payload); }

  async activity<T>(name: string, work: () => T | Promise<T>): Promise<T> {
    const subjectId = uuidV7(); this.emit("activity", "start", name, {}, undefined, subjectId);
    try { const value = await work(); this.emit("activity", "end", name, { outcome: "success" }, undefined, subjectId); return value; }
    catch (error) { this.emit("activity", "end", name, { outcome: error instanceof DOMException && error.name === "AbortError" ? "cancelled" : "failure", error_type: error instanceof Error ? error.name : "unknown" }, undefined, subjectId); throw error; }
  }

  async flush(): Promise<number> {
    if (!this.enabled || this.consent !== "granted") return 0;
    const records = this.buffer.peek(200); if (records.length === 0) return 0;
    await this.exporter.export(records); this.buffer.remove(records.length); return records.length;
  }

  stop(): void { if (this.enabled && this.consent === "granted") this.emit("session", "end", "app.session", {}); }

  private emit(kind: BehaviorKind, operation: Operation, name: string, payload: Readonly<Record<string, unknown>>, page = this.page, subjectId?: string): void {
    if (!this.enabled || this.consent !== "granted") return;
    if (!SEMANTIC_NAME.test(name)) { this.diagnostic("invalid_name", `Rejected non-semantic name: ${name}`); return; }
    const now = this.config.now?.() ?? Date.now(); const recordId = uuidV7(now); this.sequence += 1;
    const resolvedSubjectId = subjectId ?? (operation === "instant" ? recordId : uuidV7(now));
    const context: BehaviorRecord["context"] = page ? { session_id: this.sessionId, replay_id: this.replayId, page } : { session_id: this.sessionId, replay_id: this.replayId };
    const base = {
      schema_version: "1.0.0" as const, schema_url: "https://schemas.chill.dev/behavior/v1/envelope.schema.json" as const,
      record_id: recordId, subject_id: resolvedSubjectId, kind, operation, name,
      clock: { wall_unix_nano: `${Math.trunc(now)}000000`, monotonic_nano: `${Math.trunc(performance.now() * 1_000_000)}`, sequence_number: this.sequence },
      source: { platform: "web" as const, sdk_version: "0.1.0" as const, installation_id: this.installationId, page_url: globalThis.location?.href ?? "about:blank" },
      context, annotations: this.context.toJSON(), annotation_classifications: Object.fromEntries(this.context.classifications), privacy: { consent: this.consent, policy_version: this.config.policyVersion, capture_class: kind === "replay" ? "replay" as const : "analytics" as const, redaction_state: kind === "replay" ? "applied" as const : "none" as const }, payload,
    };
    const record: BehaviorRecord = this.currentTrace ? { ...base, trace: this.currentTrace } : base;
    this.buffer.push(record);
  }

  private captureContextDiagnostics(context: AnnotationContext): void { for (const diagnostic of context.diagnostics) this.diagnostic(diagnostic.code, `${diagnostic.key} kept outer value`); }
  private diagnostic(code: string, message: string): void { this.diagnostics.push({ code, message, at: Date.now() }); if (this.diagnostics.length > 100) this.diagnostics.shift(); }
}

function pagePayload(page: PageIdentity, operation: "start" | "end", cause: "initial" | "navigate"): Readonly<Record<string, unknown>> {
  return {
    surface_id: page.surfaceId,
    instance_id: page.instanceId,
    path: page.path,
    path_instance_ids: page.pathInstanceIds,
    relation: "root",
    exposure: operation === "start" ? "visible" : "retained",
    focused: operation === "start",
    cause,
  };
}

export function instrumentActivity<Arguments extends unknown[], Result>(client: ChillBrowser, name: string, work: (...args: Arguments) => Result | Promise<Result>): (...args: Arguments) => Promise<Result> {
  return (...args) => client.activity(name, () => work(...args));
}
