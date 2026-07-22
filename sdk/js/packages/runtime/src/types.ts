export type SemanticName = `${string}.${string}`;
export type Traceparent = `00-${string}-${string}-${string}`;
export type Consent = "granted" | "denied";
export type AnnotationValue = string | number | boolean;
export interface Event<N extends SemanticName = SemanticName> { name: N; kind?: "event" | "action"; annotations?: Record<string, AnnotationValue>; timestamp?: number }
export interface ActivityContext { traceparent: Traceparent; spanId: string; sessionId: string }
type CommonOptions = { serviceName: string; policyVersion?: string; installationId?: string; runtimeName?: string; trustedOrigins?: readonly string[]; annotationKeys?: readonly string[]; maxAnnotations?: number; maxQueue?: number; flushIntervalMs?: number; storePath?: string; fetch?: typeof fetch };
export type RuntimeOptions = CommonOptions & ({ endpoint: string; sdkKey: string } | { endpoint?: undefined; sdkKey?: undefined });
export interface Diagnostic { kind: "lifecycle" | "crash" | "queue" | "storage"; timestamp: number; detail?: "started" | "stopped" | "flush_failed" | "queue_full" | "storage_failed" }
