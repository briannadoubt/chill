import { prependCarrier } from "./carrier.js";
import { assertSemanticName } from "./main.js";
import type { BrowserClient, ChillPort, Outcome } from "./port.js";
export interface IpcRendererLike { invoke(channel: string, ...args: unknown[]): Promise<unknown>; send(channel: string, ...args: unknown[]): void; }
export interface RendererOptions { port: ChillPort; browser: BrowserClient; now?: () => number; }
const MAX_DURATION_MS = 60_000;
export class ChillElectronRenderer {
  private readonly now: () => number;
  constructor(private readonly ipc: IpcRendererLike, private readonly options: RendererOptions) { this.now = options.now ?? Date.now; }
  async invoke(semanticName: string, channel: string, ...args: unknown[]): Promise<unknown> {
    assertSemanticName(semanticName);
    const started = this.now();
    try { const result = await this.ipc.invoke(channel, ...prependCarrier(args, this.options.port.currentCarrierTraceparent())); this.record(semanticName, "invoke", "ok", started); return result; }
    catch (error) { this.record(semanticName, "invoke", "error", started); throw error; }
  }
  send(semanticName: string, channel: string, ...args: unknown[]): void { assertSemanticName(semanticName); const started = this.now(); try { this.ipc.send(channel, ...prependCarrier(args, this.options.port.currentCarrierTraceparent())); this.record(semanticName, "send", "ok", started); } catch (error) { this.record(semanticName, "send", "error", started); throw error; } }
  recordFailure(): void { this.options.browser.recordStableFact("electron_renderer_failure"); }
  private record(name: string, style: string, outcome: Outcome, started: number): void { this.options.port.record(name, { direction: "renderer_to_main", style, outcome, duration_ms: Math.min(MAX_DURATION_MS, Math.max(0, this.now() - started)) }); }
}
