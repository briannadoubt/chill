import type { Traceparent } from "./types.js";
const bytes = (n: number) => { const out = new Uint8Array(n); crypto.getRandomValues(out); return out; };
const hex = (data: Uint8Array) => [...data].map(value => value.toString(16).padStart(2, "0")).join("");
export function uuidv4(): string { const b = bytes(16); b[6] = (b[6]! & 15) | 64; b[8] = (b[8]! & 63) | 128; const h = hex(b); return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`; }
export function uuidv7(): string { const b = bytes(16); let ms = Date.now(); for (let i = 5; i >= 0; i--) { b[i] = ms & 255; ms = Math.floor(ms / 256); } b[6] = (b[6]! & 15) | 112; b[8] = (b[8]! & 63) | 128; const h = hex(b); return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`; }
export function newTraceparent(): Traceparent { return `00-${hex(bytes(16))}-${hex(bytes(8))}-01`; }
export function isTraceparent(value: string | null | undefined): value is Traceparent { if (!value || !/^00-[0-9a-f]{32}-[0-9a-f]{16}-0[01]$/.test(value)) return false; const [_, trace, span] = value.split("-"); return !/^0+$/.test(trace!) && !/^0+$/.test(span!); }
export const isUuidV4 = (v: string) => /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(v);
