import type { ChillBrowser } from "./runtime.js";

interface ReplayNode { tag: string; id?: string; role?: string; masked: true; children: ReplayNode[]; }
function serialize(element: Element): ReplayNode {
  const node: ReplayNode = { tag: element.tagName.toLowerCase(), masked: true, children: [...element.children].map(serialize) };
  const id = (element as HTMLElement).dataset.chillId; if (id) node.id = id;
  const role = element.getAttribute("role"); if (role) node.role = role;
  return node;
}

export function startReplay(client: ChillBrowser, root: HTMLElement = document.documentElement): () => void {
  if (!client.shouldReplay()) return () => undefined;
  let sequence = 0; let dropped = 0; const maximumChunks = 1_000; const started = performance.now();
  const capture = (type: string, payload: Record<string, unknown>) => { if (sequence >= maximumChunks) { dropped += 1; return; } client.replay({ replay_id: client.replayId, sequence: sequence++, monotonic_offset_nano: `${Math.trunc((performance.now() - started) * 1_000_000)}`, type, masking: "source", ...payload }); };
  capture("snapshot", { tree: serialize(root), viewport: { width: innerWidth, height: innerHeight } });
  const observer = new MutationObserver(records => capture("mutation", { mutations: records.map(record => ({ type: record.type, target: record.target instanceof Element ? record.target.tagName.toLowerCase() : "text", added_count: record.addedNodes.length, removed_count: record.removedNodes.length })) }));
  observer.observe(root, { subtree: true, childList: true, attributes: true });
  const pointer = (event: PointerEvent) => capture("gesture", { gesture: event.type, x: Math.round(event.clientX), y: Math.round(event.clientY) });
  const scroll = () => capture("scroll", { x: Math.round(scrollX), y: Math.round(scrollY) });
  root.addEventListener("pointerdown", pointer, true); addEventListener("scroll", scroll, { capture: true, passive: true });
  return () => { observer.disconnect(); root.removeEventListener("pointerdown", pointer, true); removeEventListener("scroll", scroll, true); if (dropped > 0) client.replay({ replay_id: client.replayId, sequence, type: "gap", dropped_chunks: dropped, reason: "capacity", masking: "source" }); };
}
