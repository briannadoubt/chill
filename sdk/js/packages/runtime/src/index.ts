import { AsyncLocalStorage } from "node:async_hooks";

import { FileStore, MemoryStore, type RecordStore } from "./store.js";
import {
  isTraceparent,
  isUuidV4,
  newTraceparent,
  uuidv4,
  uuidv7,
} from "./trace.js";
import type {
  ActivityContext,
  AnnotationValue,
  Consent,
  Diagnostic,
  Event,
  RuntimeOptions,
  SemanticName,
  Traceparent,
} from "./types.js";

export * from "./types.js";
export { isTraceparent, newTraceparent, uuidv4, uuidv7 } from "./trace.js";

const SCHEMA = "https://schemas.chill.dev/behavior/v1/envelope.schema.json";
const SEMANTIC_NAME = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/;
const ANNOTATION_KEY = /^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$/;
const MAX_SAFE_SEQUENCE = 9_007_199_254_740_991;

const nano = () => (BigInt(Date.now()) * 1_000_000n).toString();
const attr = (key: string, value: AnnotationValue) => ({
  key,
  value:
    typeof value === "string"
      ? { stringValue: value }
      : typeof value === "boolean"
        ? { boolValue: value }
        : Number.isInteger(value)
          ? { intValue: String(value) }
          : { doubleValue: value },
});

type Stored = { log: Record<string, unknown> };
type BehaviorKind = "session" | "action" | "activity" | "event";
type BehaviorOperation = "instant" | "start" | "end";

function endpointIsAllowed(url: URL): boolean {
  const loopback =
    url.hostname === "localhost" ||
    url.hostname === "127.0.0.1" ||
    url.hostname === "[::1]";
  return (url.protocol === "https:" || (url.protocol === "http:" && loopback)) &&
    url.username === "" &&
    url.password === "" &&
    url.pathname === "/v1/logs" &&
    url.search === "" &&
    url.hash === "";
}

function payloadAttributes(
  kind: BehaviorKind,
  name: string,
): ReturnType<typeof attr>[] {
  switch (kind) {
    case "activity":
      return [
        attr("chill.payload.activity_kind", "custom"),
        attr("chill.payload.role", "operation"),
        attr("chill.payload.attempt", 1),
        attr("chill.payload.recursion_depth", 0),
      ];
    case "action":
      return [
        attr("chill.payload.element_id", name),
        attr("chill.payload.role", "operation"),
        attr("chill.payload.activation", "system"),
        attr("chill.payload.input", "system"),
      ];
    case "event":
      return [
        attr("chill.payload.event_class", "custom"),
        attr("chill.payload.emission", "observed"),
      ];
    case "session":
      return [];
  }
}

export class ChillRuntime {
  private readonly context = new AsyncLocalStorage<ActivityContext>();
  private readonly store: RecordStore;
  private readonly allowed: Set<string>;
  private readonly endpoint?: string;
  private readonly sdkKey?: string;
  private readonly sessionId = uuidv7();
  private readonly processId = uuidv4();
  private sequence = 0;
  private consent: Consent = "denied";
  private timer?: ReturnType<typeof setInterval>;
  private aborter = new AbortController();
  private started = false;
  private sessionOpen = false;

  readonly diagnostics: Diagnostic[] = [];
  readonly resource: Record<string, string>;
  readonly installationId: string;
  readonly policyVersion: string;

  constructor(private readonly options: RuntimeOptions) {
    if (options.serviceName.length < 1 || options.serviceName.length > 128) {
      throw new Error("invalid serviceName");
    }
    if (options.endpoint) {
      const url = new URL(options.endpoint);
      if (
        !endpointIsAllowed(url) ||
        !options.sdkKey ||
        options.sdkKey.length > 512
      ) {
        throw new Error("endpoint must be an allowed /v1/logs URL with sdkKey");
      }
      this.endpoint = url.href;
      this.sdkKey = options.sdkKey;
    }
    this.installationId = options.installationId ?? uuidv4();
    if (!isUuidV4(this.installationId)) {
      throw new Error("installationId must be lowercase UUIDv4");
    }
    this.policyVersion = options.policyVersion ?? "privacy-v1";
    if (this.policyVersion.length < 1 || this.policyVersion.length > 64) {
      throw new Error("invalid policyVersion");
    }
    const annotationKeys = options.annotationKeys ?? [];
    if (annotationKeys.some((key) => !ANNOTATION_KEY.test(key) || key.length > 128)) {
      throw new Error("invalid annotation key");
    }
    this.allowed = new Set(annotationKeys);
    this.store = options.storePath
      ? new FileStore(options.storePath)
      : new MemoryStore();
    this.resource = {
      "service.name": options.serviceName,
      "telemetry.sdk.language": "javascript",
      "process.runtime.name": options.runtimeName ?? "javascript",
      "process.runtime.version":
        typeof process === "undefined" ? "unknown" : process.version,
      "os.type": typeof process === "undefined" ? "unknown" : process.platform,
      "chill.source.platform": "server",
      "chill.source.installation_id": this.installationId,
      "chill.source.process_id": this.processId,
    };
  }

  private diag(kind: Diagnostic["kind"], detail: Diagnostic["detail"]): void {
    if (this.diagnostics.length >= 64) this.diagnostics.shift();
    this.diagnostics.push({ kind, timestamp: Date.now(), detail });
  }

  private valid(name: string): boolean {
    return SEMANTIC_NAME.test(name) && name.length <= 128;
  }

  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    this.diag("lifecycle", "started");
    if (this.consent === "granted") {
      this.sessionOpen = await this.emit(
        "session",
        "start",
        "session.started",
        this.sessionId,
        {},
      );
    }
    if (this.endpoint) {
      this.timer = setInterval(
        () => void this.flush(),
        this.options.flushIntervalMs ?? 10_000,
      );
    }
  }

  async stop(): Promise<void> {
    if (!this.started) return;
    if (this.timer) clearInterval(this.timer);
    if (this.consent === "granted" && this.sessionOpen) {
      await this.emit("session", "end", "session.ended", this.sessionId, {});
      this.sessionOpen = false;
    }
    await this.flush();
    this.started = false;
    this.diag("lifecycle", "stopped");
  }

  async setConsent(consent: Consent): Promise<void> {
    if (consent === this.consent) return;
    this.consent = consent;
    if (consent === "granted" && this.started && !this.sessionOpen) {
      this.sessionOpen = await this.emit(
        "session",
        "start",
        "session.started",
        this.sessionId,
        {},
      );
    }
    if (consent === "denied") {
      this.sessionOpen = false;
      this.aborter.abort();
      this.aborter = new AbortController();
      try {
        await this.store.purge();
      } catch {
        this.diag("storage", "storage_failed");
      }
    }
  }

  get signal(): AbortSignal {
    return this.aborter.signal;
  }

  current(): ActivityContext | undefined {
    return this.context.getStore();
  }

  async activity<T>(
    name: SemanticName,
    work: () => T | Promise<T>,
    parent?: string,
  ): Promise<T> {
    if (!this.valid(name)) throw new Error("invalid semantic name");
    if (this.consent !== "granted") return await work();
    const base = isTraceparent(parent)
      ? parent
      : this.current()?.traceparent ?? newTraceparent();
    const child = newTraceparent();
    const traceparent = `00-${base.slice(3, 35)}-${child.slice(36, 52)}-01` as Traceparent;
    const context = {
      traceparent,
      spanId: traceparent.slice(36, 52),
      sessionId: this.current()?.sessionId ?? this.sessionId,
    };
    const subject = uuidv7();
    const begun = performance.now();
    await this.context.run(context, () =>
      this.emit("activity", "start", name, subject, {}),
    );
    try {
      const value = await this.context.run(context, work);
      await this.context.run(context, () =>
        this.emit(
          "activity",
          "end",
          name,
          subject,
          {},
          "ok",
          Math.max(0, performance.now() - begun) * 1e6,
        ),
      );
      return value;
    } catch (error) {
      await this.context.run(context, () =>
        this.emit(
          "activity",
          "end",
          name,
          subject,
          {},
          "error",
          Math.max(0, performance.now() - begun) * 1e6,
        ),
      );
      throw error;
    }
  }

  async record<N extends SemanticName>(event: Event<N>): Promise<boolean> {
    if (
      !this.valid(event.name) ||
      Object.values(event.annotations ?? {}).some(
        (value) =>
          (typeof value === "number" && !Number.isFinite(value)) ||
          (typeof value === "string" && value.length > 1024),
      )
    ) {
      throw new Error("invalid event");
    }
    if (this.consent !== "granted") return false;
    return this.emit(
      event.kind ?? "event",
      "instant",
      event.name,
      undefined,
      event.annotations ?? {},
      undefined,
      undefined,
      event.timestamp,
    );
  }

  private async emit(
    kind: BehaviorKind,
    operation: BehaviorOperation,
    name: string,
    subjectId: string | undefined,
    raw: Record<string, AnnotationValue>,
    outcome?: "ok" | "error",
    duration?: number,
    timestamp?: number,
  ): Promise<boolean> {
    if (this.consent !== "granted") return false;
    if (this.sequence >= MAX_SAFE_SEQUENCE) {
      this.diag("queue", "queue_full");
      return false;
    }
    const kept: Record<string, AnnotationValue> = {};
    for (const [key, value] of Object.entries(raw)) {
      if (
        this.allowed.has(key) &&
        Object.keys(kept).length < (this.options.maxAnnotations ?? 16)
      ) {
        kept[key] = value;
      }
    }
    const id = uuidv7();
    const subject = subjectId ?? id;
    const context = this.current();
    const traceparent = context?.traceparent ?? newTraceparent();
    const occurredAt =
      timestamp === undefined
        ? nano()
        : String(BigInt(Math.floor(timestamp)) * 1_000_000n);
    const attributes = [
      attr("chill.schema.version", "1.0.0"),
      attr("chill.schema.url", SCHEMA),
      attr("chill.record.id", id),
      attr("chill.subject.id", subject),
      attr("chill.behavior.kind", kind),
      attr("chill.behavior.operation", operation),
      attr("chill.behavior.name", name),
      attr("chill.clock.sequence_number", ++this.sequence),
      attr("chill.source.platform", "server"),
      attr("chill.source.installation_id", this.installationId),
      attr("chill.source.process_id", this.processId),
      attr("chill.context.session_id", context?.sessionId ?? this.sessionId),
      attr("chill.privacy.consent", "granted"),
      attr("chill.privacy.policy_version", this.policyVersion),
      attr("chill.privacy.capture_class", "analytics"),
      attr("chill.privacy.redaction_state", "none"),
      ...payloadAttributes(kind, name),
      ...(outcome ? [attr("chill.outcome.status", outcome)] : []),
      ...(duration !== undefined
        ? [attr("chill.duration_nano", String(Math.floor(duration)))]
        : []),
      ...Object.entries(kept).flatMap(([key, value]) => [
        attr(`chill.annotation.${key}`, value),
        attr(`chill.privacy.annotation_classification.${key}`, "internal"),
      ]),
    ];
    const log = {
      timeUnixNano: occurredAt,
      observedTimeUnixNano: occurredAt,
      eventName: name,
      traceId: traceparent.slice(3, 35),
      spanId: traceparent.slice(36, 52),
      flags: 1,
      attributes,
    };
    try {
      const rows = await this.store.load();
      if (rows.length >= (this.options.maxQueue ?? 1000)) {
        this.diag("queue", "queue_full");
        return false;
      }
      await this.store.append(JSON.stringify({ log } satisfies Stored));
      return true;
    } catch {
      this.diag("storage", "storage_failed");
      return false;
    }
  }

  async flush(): Promise<boolean> {
    if (this.consent !== "granted" || !this.endpoint || !this.sdkKey) {
      return false;
    }
    let rows: string[];
    try {
      rows = await this.store.load();
    } catch {
      this.diag("storage", "storage_failed");
      return false;
    }
    if (!rows.length) return true;
    try {
      const logs = rows.map((row) => (JSON.parse(row) as Stored).log);
      const body = JSON.stringify({
        resourceLogs: [
          {
            resource: {
              attributes: Object.entries(this.resource).map(([key, value]) =>
                attr(key, value),
              ),
            },
            scopeLogs: [
              {
                scope: { name: "dev.chill.runtime", version: "0.1.0" },
                schemaUrl: SCHEMA,
                logRecords: logs,
              },
            ],
          },
        ],
      });
      const response = await (this.options.fetch ?? fetch)(this.endpoint, {
        method: "POST",
        headers: {
          authorization: `Bearer ${this.sdkKey}`,
          "content-type": "application/json",
          "x-chill-schema-version": "1.0.0",
        },
        body,
        signal: this.signal,
      });
      if (!response.ok) throw new Error("rejected");
      await this.store.ack(rows.length);
      return true;
    } catch {
      this.diag("lifecycle", "flush_failed");
      return false;
    }
  }

  fetch(input: string | URL, init: RequestInit = {}): Promise<Response> {
    const url = new URL(String(input));
    const headers = new Headers(init.headers);
    const traceparent = this.current()?.traceparent;
    if ((this.options.trustedOrigins ?? []).includes(url.origin) && traceparent) {
      headers.set("traceparent", traceparent);
    }
    return (this.options.fetch ?? fetch)(input, {
      ...init,
      headers,
      signal: init.signal ?? this.signal,
    });
  }

  crash(): void {
    this.diag("crash", undefined);
  }
}

export const createRuntime = (options: RuntimeOptions) => new ChillRuntime(options);
