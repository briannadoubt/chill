import { AnnotationContext, annotationKey } from "./types.js";
import type { ChillBrowser } from "./runtime.js";

function semanticName(value: string | undefined, fallback: string): string { return value && /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/.test(value) ? value : fallback; }
function elementPayload(element: HTMLElement): Record<string, unknown> { return { element_id: element.id || element.dataset.chillId || "anonymous", role: element.getAttribute("role") ?? element.tagName.toLowerCase(), input: "pointer" }; }

export function contextForElement(element: Element): AnnotationContext {
  const ancestry: Element[] = []; for (let current: Element | null = element; current; current = current.parentElement) ancestry.unshift(current);
  let context = AnnotationContext.empty();
  for (const node of ancestry) {
    const raw = (node as HTMLElement).dataset.chillAnnotations; if (!raw) continue;
    try { const values = JSON.parse(raw) as Record<string, unknown>; for (const [key, value] of Object.entries(values)) if (typeof value === "string" || typeof value === "number" || typeof value === "boolean" || (Array.isArray(value) && value.every(item => typeof item === "string"))) context = context.with(annotationKey(key), value as string); }
    catch { /* Invalid declarative metadata is ignored and never reads element content. */ }
  }
  return context;
}

export function instrumentDOM(client: ChillBrowser, root: Document | HTMLElement = document): () => void {
  const seen = new WeakSet<Element>();
  const inspect = (element: Element) => {
    if (seen.has(element)) return; seen.add(element);
    const html = element as HTMLElement;
    if (html.dataset.chillPage) client.startPage(semanticName(html.dataset.chillPage, "web.page"));
    if (html.dataset.chillImpression) { client.setContext(contextForElement(element)); client.impression(semanticName(html.dataset.chillImpression, "element.impression"), elementPayload(html)); }
  };
  root.querySelectorAll("[data-chill-page],[data-chill-impression]").forEach(inspect);
  const click = (event: Event) => { const element = (event.target as Element | null)?.closest<HTMLElement>("[data-chill-action]"); if (!element) return; client.setContext(contextForElement(element)); client.observeAction(semanticName(element.dataset.chillAction, "element.activate"), `${event.timeStamp}:${event.type}`, client.getPage()?.instanceId ?? "document", elementPayload(element)); };
  const change = (event: Event) => { const element = (event.target as Element | null)?.closest<HTMLElement>("[data-chill-form]"); if (!element) return; client.setContext(contextForElement(element)); client.event(semanticName(element.dataset.chillForm, "form.change"), { element_id: element.id || "anonymous", value_captured: false }); };
  root.addEventListener("click", click, true); root.addEventListener("change", change, true);
  const observer = new MutationObserver(records => { for (const record of records) for (const node of record.addedNodes) if (node instanceof Element) { inspect(node); node.querySelectorAll("[data-chill-page],[data-chill-impression]").forEach(inspect); } });
  observer.observe(root, { childList: true, subtree: true });
  const error = (event: ErrorEvent) => client.event("browser.error", { error_type: event.error instanceof Error ? event.error.name : "Error" });
  const rejection = (event: PromiseRejectionEvent) => client.event("browser.promise_rejection", { error_type: event.reason instanceof Error ? event.reason.name : "unknown" });
  const visibility = () => client.event("browser.visibility", { state: document.visibilityState });
  globalThis.addEventListener?.("error", error);
  globalThis.addEventListener?.("unhandledrejection", rejection); document.addEventListener?.("visibilitychange", visibility);
  let performanceObserver: PerformanceObserver | undefined;
  if (typeof PerformanceObserver !== "undefined") { try { performanceObserver = new PerformanceObserver(list => { for (const entry of list.getEntries()) client.event("browser.performance", { entry_type: entry.entryType, duration_ms: entry.duration, name: entry.name.slice(0, 128) }); }); performanceObserver.observe({ entryTypes: ["navigation", "longtask", "largest-contentful-paint"] }); } catch { /* Unsupported entry types are optional. */ } }
  return () => { observer.disconnect(); performanceObserver?.disconnect(); root.removeEventListener("click", click, true); root.removeEventListener("change", change, true); globalThis.removeEventListener?.("error", error); globalThis.removeEventListener?.("unhandledrejection", rejection); document.removeEventListener?.("visibilitychange", visibility); };
}
