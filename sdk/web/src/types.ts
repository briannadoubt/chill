export type AnnotationPrimitive = string | number | boolean | readonly string[];

export interface AnnotationKey<T extends AnnotationPrimitive> {
  readonly name: string;
  readonly classification: "public" | "internal" | "pseudonymous_identifier";
  readonly validate?: (value: unknown) => value is T;
}

export function annotationKey<T extends AnnotationPrimitive>(
  name: string,
  classification: AnnotationKey<T>["classification"] = "internal",
  validate?: AnnotationKey<T>["validate"],
): AnnotationKey<T> {
  if (!/^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$/.test(name)) throw new TypeError(`invalid annotation key: ${name}`);
  return validate ? { name, classification, validate } : { name, classification };
}

export interface ContextDiagnostic {
  readonly code: "annotation_collision" | "annotation_invalid";
  readonly key: string;
  readonly kept?: AnnotationPrimitive;
  readonly rejected: unknown;
}

export class AnnotationContext {
  readonly values: ReadonlyMap<string, AnnotationPrimitive>;
  readonly classifications: ReadonlyMap<string, AnnotationKey<AnnotationPrimitive>["classification"]>;
  readonly diagnostics: readonly ContextDiagnostic[];

  private constructor(
    values: ReadonlyMap<string, AnnotationPrimitive>,
    classifications: ReadonlyMap<string, AnnotationKey<AnnotationPrimitive>["classification"]>,
    diagnostics: readonly ContextDiagnostic[],
  ) {
    this.values = values;
    this.classifications = classifications;
    this.diagnostics = diagnostics;
  }

  static empty(): AnnotationContext { return new AnnotationContext(new Map(), new Map(), []); }

  with<T extends AnnotationPrimitive>(key: AnnotationKey<T>, value: T): AnnotationContext {
    if (key.validate && !key.validate(value)) {
      return new AnnotationContext(this.values, this.classifications, [...this.diagnostics, { code: "annotation_invalid", key: key.name, rejected: value }]);
    }
    if (this.values.has(key.name)) {
      return new AnnotationContext(this.values, this.classifications, [...this.diagnostics, { code: "annotation_collision", key: key.name, kept: this.values.get(key.name)!, rejected: value }]);
    }
    const values = new Map(this.values); values.set(key.name, value);
    const classifications = new Map(this.classifications); classifications.set(key.name, key.classification);
    return new AnnotationContext(values, classifications, this.diagnostics);
  }

  mergeDescendant(descendant: AnnotationContext): AnnotationContext {
    let merged: AnnotationContext = this;
    for (const [name, value] of descendant.values) {
      merged = merged.with(annotationKey(name, descendant.classifications.get(name) ?? "internal"), value);
    }
    return new AnnotationContext(merged.values, merged.classifications, [...merged.diagnostics, ...descendant.diagnostics]);
  }

  toJSON(): Record<string, AnnotationPrimitive> { return Object.fromEntries(this.values); }
}

export type BehaviorKind = "session" | "journey" | "page" | "impression" | "action" | "activity" | "event" | "replay";
export type Operation = "instant" | "start" | "update" | "end";
export interface PageIdentity { readonly surfaceId: string; readonly instanceId: string; readonly path: readonly string[]; readonly pathInstanceIds: readonly string[]; }
export interface TraceContext { readonly traceId: string; readonly spanId: string; readonly sampled: boolean; }

export interface BehaviorRecord {
  readonly schema_version: "1.0.0";
  readonly schema_url: "https://schemas.chill.dev/behavior/v1/envelope.schema.json";
  readonly record_id: string;
  readonly subject_id: string;
  readonly kind: BehaviorKind;
  readonly operation: Operation;
  readonly name: string;
  readonly clock: { readonly wall_unix_nano: string; readonly monotonic_nano: string; readonly sequence_number: number };
  readonly source: { readonly platform: "web"; readonly sdk_version: "0.1.0"; readonly installation_id: string; readonly page_url: string };
  readonly context: { readonly session_id: string; readonly page?: PageIdentity; readonly replay_id?: string };
  readonly annotations: Record<string, AnnotationPrimitive>;
  readonly annotation_classifications: Record<string, AnnotationKey<AnnotationPrimitive>["classification"]>;
  readonly privacy: { readonly consent: Consent; readonly policy_version: string; readonly capture_class: "analytics" | "replay"; readonly redaction_state: "none" | "applied" };
  readonly trace?: TraceContext;
  readonly payload: Readonly<Record<string, unknown>>;
}

export type Consent = "unknown" | "denied" | "granted";
export interface RuntimeConfiguration {
  readonly endpoint: string;
  readonly sdkKey: string;
  readonly policyVersion: string;
  readonly installationId?: string;
  readonly consent?: Consent;
  readonly sampleRate?: number;
  readonly replaySampleRate?: number;
  readonly maxBufferedRecords?: number;
  readonly allowedAnnotationKeys?: readonly string[];
  readonly allowedPayloadKeys?: readonly string[];
  readonly trustedTraceOrigins?: readonly string[];
  readonly fetch?: typeof globalThis.fetch;
  readonly storage?: Storage;
  readonly now?: () => number;
  readonly random?: () => number;
}
