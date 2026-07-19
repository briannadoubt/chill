import { randomHex } from "./runtime.js";
import type { ChillBrowser } from "./runtime.js";
import type { TraceContext } from "./types.js";

export function parseTraceparent(value: string | null): TraceContext | undefined {
  const match = /^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/.exec(value ?? "");
  return match?.[1] && match[2] && match[3] && !/^0+$/.test(match[1]) && !/^0+$/.test(match[2]) ? { traceId: match[1], spanId: match[2], sampled: (Number.parseInt(match[3], 16) & 1) === 1 } : undefined;
}

export function instrumentXMLHttpRequest(client: ChillBrowser, trustedOrigins: readonly string[], constructor: typeof XMLHttpRequest = XMLHttpRequest): () => void {
  const trusted = new Set(trustedOrigins.map(origin => new URL(origin).origin)); const metadata = new WeakMap<XMLHttpRequest, { method: string; url: URL; started?: number }>();
  type Open = (this: XMLHttpRequest, method: string, url: string | URL, async?: boolean, username?: string | null, password?: string | null) => void;
  type Send = (this: XMLHttpRequest, body?: Document | XMLHttpRequestBodyInit | null) => void;
  const prototype = constructor.prototype as unknown as { open: Open; send: Send }; const open = prototype.open; const send = prototype.send;
  prototype.open = function(method, url, async = true, username, password): void { metadata.set(this, { method, url: new URL(url, location.href) }); open.call(this, method, url, async, username, password); };
  prototype.send = function(body): void { const data = metadata.get(this); if (data) { data.started = performance.now(); if (trusted.has(data.url.origin)) this.setRequestHeader("traceparent", traceparent(child(client.getTrace()))); this.addEventListener("loadend", () => client.event("http.client", { method: data.method, origin: data.url.origin, status_code: this.status, duration_ms: performance.now() - (data.started ?? performance.now()), trace_propagated: trusted.has(data.url.origin) }), { once: true }); } send.call(this, body); };
  return () => { prototype.open = open; prototype.send = send; };
}
export function traceparent(context: TraceContext): string { return `00-${context.traceId}-${context.spanId}-${context.sampled ? "01" : "00"}`; }
function child(parent?: TraceContext): TraceContext { return { traceId: parent?.traceId ?? randomHex(16), spanId: randomHex(8), sampled: parent?.sampled ?? true }; }

export function instrumentFetch(client: ChillBrowser, trustedOrigins: readonly string[], original: typeof fetch = fetch): typeof fetch {
  const trusted = new Set(trustedOrigins.map(origin => new URL(origin).origin));
  return async (input, init) => {
    const url = new URL(input instanceof Request ? input.url : input.toString(), location.href); const context = child(client.getTrace());
    const headers = new Headers(input instanceof Request ? input.headers : init?.headers);
    if (trusted.has(url.origin)) headers.set("traceparent", traceparent(context));
    const started = performance.now();
    try { const response = await original(input, { ...init, headers }); client.event("http.client", { method: init?.method ?? (input instanceof Request ? input.method : "GET"), origin: url.origin, status_code: response.status, duration_ms: performance.now() - started, trace_propagated: trusted.has(url.origin) }); return response; }
    catch (error) { client.event("http.client", { method: init?.method ?? "GET", origin: url.origin, error_type: error instanceof Error ? error.name : "unknown", duration_ms: performance.now() - started }); throw error; }
  };
}

export function instrumentNavigation(client: ChillBrowser): () => void {
  const push = history.pushState.bind(history); const replace = history.replaceState.bind(history);
  const capture = () => client.startPage(semanticRoute(location.pathname));
  history.pushState = ((...args: Parameters<History["pushState"]>) => { push(...args); capture(); }) as History["pushState"];
  history.replaceState = ((...args: Parameters<History["replaceState"]>) => { replace(...args); capture(); }) as History["replaceState"];
  addEventListener("popstate", capture);
  return () => { history.pushState = push; history.replaceState = replace; removeEventListener("popstate", capture); };
}
function semanticRoute(pathname: string): string { const value = pathname.split("/").filter(Boolean).at(-1)?.toLowerCase().replace(/[^a-z0-9_-]/g, "-") ?? "root"; return /^[a-z]/.test(value) ? value : `route-${value}`; }
