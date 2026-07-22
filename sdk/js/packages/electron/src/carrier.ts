const PREFIX = "__chill_ipc_v1__";
const TRACEPARENT = /^00-([0-9a-f]{32})-([0-9a-f]{16})-(0[01])$/;
const UUIDV7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

export interface Carrier { readonly traceparent: string; readonly messageId: string; }
export type CarrierResult = { readonly carrier?: Carrier; readonly args: readonly unknown[] };

/** UUIDv7-shaped, time-sortable id.  It intentionally carries no app data. */
export interface CryptoRandom { getRandomValues(values: Uint8Array): Uint8Array; }
export function uuidv7(now = Date.now(), crypto: CryptoRandom = globalThis.crypto): string {
  if (!crypto?.getRandomValues) throw new Error("cryptographically secure randomness is required");
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  const hex = (n: number, width: number) => n.toString(16).padStart(width, "0");
  const time = hex(now, 12);
  const a = hex((bytes[0] << 4) | (bytes[1] >> 4), 3);
  const b = hex(0x8000 | ((bytes[1] & 0x0f) << 8) | bytes[2], 4);
  const c = [...bytes.slice(3, 9)].map((byte) => hex(byte, 2)).join("");
  return `${time.slice(0, 8)}-${time.slice(8)}-7${a}-${b}-${c}`;
}

export function prependCarrier(args: readonly unknown[], traceparent: string, messageId = uuidv7()): unknown[] {
  if (!validTraceparent(traceparent)) throw new TypeError("traceparent must be strict W3C version 00");
  if (!UUIDV7.test(messageId)) throw new TypeError("messageId must be UUIDv7");
  return [{ [PREFIX]: { traceparent, messageId } }, ...args];
}

export function stripCarrier(args: readonly unknown[]): CarrierResult {
  const first = args[0];
  if (first === null || typeof first !== "object" || Array.isArray(first)) return { args };
  const value = (first as Record<string, unknown>)[PREFIX];
  if (value === null || typeof value !== "object" || Array.isArray(value)) return { args };
  const record = value as Record<string, unknown>;
  if (typeof record.traceparent !== "string" || typeof record.messageId !== "string" || !validTraceparent(record.traceparent) || !UUIDV7.test(record.messageId)) return { args };
  return { carrier: { traceparent: record.traceparent, messageId: record.messageId }, args: args.slice(1) };
}

export function validTraceparent(value: string): boolean {
  const match = TRACEPARENT.exec(value);
  return match !== null && !/^0+$/.test(match[1]) && !/^0+$/.test(match[2]);
}
