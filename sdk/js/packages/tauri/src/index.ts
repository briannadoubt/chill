/** Metadata-only Tauri renderer correlation. This module intentionally never
 * accesses application command arguments, results, errors, or event payloads. */
export const CARRIER_VERSION = 1 as const;
const HEADER = "x-chill-carrier";
const MAX_BAGGAGE_ENTRIES = 8;
const MAX_BAGGAGE_KEY = 32;
const MAX_BAGGAGE_VALUE = 64;

export interface TauriCarrier {
  readonly version: typeof CARRIER_VERSION;
  readonly traceparent: string;
  readonly baggage?: Readonly<Record<string, string>>;
  readonly semanticName: string;
}
export interface CarrierPolicy { readonly baggageAllowlist?: readonly string[]; }

/** An explicit wrapper sent with (not derived from) an application event payload. */
export interface TauriEventEnvelope<T> {
  readonly carrier: TauriCarrier;
  readonly payload: T;
}

export interface TauriInvokeOptions { readonly headers?: Readonly<Record<string, string>>; }
export type TauriInvoke = <T>(command: string, args?: unknown, options?: TauriInvokeOptions) => Promise<T>;

export function commandOptions(carrier: TauriCarrier, policy: CarrierPolicy = {}): TauriInvokeOptions {
  return { headers: { [HEADER]: serializeCarrier(carrier, policy) } };
}

/** The raw Tauri command is deliberately separate from the declared semantic name. */
export function invokeWithCarrier<T>(invoke: TauriInvoke, rawCommand: string, args: unknown, carrier: TauriCarrier, policy: CarrierPolicy = {}): Promise<T> {
  return invoke<T>(rawCommand, args, commandOptions(carrier, policy));
}

export function eventEnvelope<T>(carrier: TauriCarrier, payload: T): TauriEventEnvelope<T> {
  // No inspection, cloning, serialization, or logging of payload occurs here.
  return { carrier, payload };
}

export function parseCarrier(value: string, policy: CarrierPolicy = {}): TauriCarrier | undefined {
  try {
    const parsed: unknown = JSON.parse(value);
    if (!isRecord(parsed) || parsed.version !== CARRIER_VERSION || typeof parsed.traceparent !== "string" || typeof parsed.semanticName !== "string") return undefined;
    const baggage = parsed.baggage;
    if (baggage !== undefined && (!isRecord(baggage) || !Object.entries(baggage).every(([k, v]) => typeof v === "string" && baggageKey(k) && v.length <= MAX_BAGGAGE_VALUE))) return undefined;
    const carrier: TauriCarrier = { version: CARRIER_VERSION, traceparent: parsed.traceparent, semanticName: parsed.semanticName, ...(baggage === undefined ? {} : { baggage: baggage as Record<string, string> }) };
    return validCarrier(carrier, policy) ? carrier : undefined;
  } catch { return undefined; }
}

export function validCarrier(carrier: TauriCarrier, policy: CarrierPolicy = {}): boolean {
  const allowlist = new Set(policy.baggageAllowlist ?? []);
  return carrier.version === CARRIER_VERSION && traceparent(carrier.traceparent) && semanticName(carrier.semanticName) && (carrier.baggage === undefined || Object.entries(carrier.baggage).length <= MAX_BAGGAGE_ENTRIES && Object.entries(carrier.baggage).every(([key, value]) => baggageKey(key) && value.length <= MAX_BAGGAGE_VALUE && allowlist.has(key)));
}

function serializeCarrier(carrier: TauriCarrier, policy: CarrierPolicy): string {
  if (!validCarrier(carrier, policy)) throw new TypeError("invalid Chill Tauri carrier");
  return JSON.stringify(carrier);
}
function isRecord(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }
function semanticName(value: string): boolean { return value.length > 0 && value.length <= 80 && /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/.test(value); }
function baggageKey(value: string): boolean { return value.length > 0 && value.length <= MAX_BAGGAGE_KEY && /^[A-Za-z0-9._-]+$/.test(value); }
function traceparent(value: string): boolean { const match = /^00-([0-9a-f]{32})-([0-9a-f]{16})-(00|01)$/.exec(value); return match !== null && !/^0+$/.test(match[1]!) && !/^0+$/.test(match[2]!); }
