export interface ContextBridgeLike { exposeInMainWorld(name: string, api: object): void; }
export type NamedBridge = Readonly<Record<string, (...args: unknown[]) => unknown>>;
/** Exposes only call sites declared by the application; never a channel-capable bridge. */
export function exposeNamedBridge(contextBridge: ContextBridgeLike, name: string, bridge: NamedBridge): void {
  contextBridge.exposeInMainWorld(name, Object.freeze({ ...bridge }));
}
