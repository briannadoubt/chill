import { stripCarrier } from "./carrier.js";
import type { ChillPort, Outcome } from "./port.js";

export interface AppLike { on(event: string, listener: (...args: unknown[]) => void): unknown; }
export interface WebContentsLike { on(event: string, listener: (...args: unknown[]) => void): unknown; }
export interface BrowserWindowLike { webContents: WebContentsLike; on(event: string, listener: (...args: unknown[]) => void): unknown; }
export interface AutoUpdaterLike { on(event: string, listener: (...args: unknown[]) => void): unknown; }
export interface IpcMainLike { handle(channel: string, handler: (event: unknown, ...args: unknown[]) => unknown): void; }
export interface MainOptions { port: ChillPort; now?: () => number; }
export interface IpcDeclaration { channel: string; semanticName: string; handler: (event: unknown, ...args: unknown[]) => unknown; }
const SEMANTIC_NAME = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/;
const MAX_DURATION_MS = 60_000;

export class ChillElectronMain {
  private readonly now: () => number;
  constructor(private readonly options: MainOptions) { this.now = options.now ?? Date.now; }
  observeApp(app: AppLike): void { for (const event of ["ready", "before-quit", "will-quit", "quit", "child-process-gone"]) app.on(event, () => this.fact("electron.lifecycle", event === "child-process-gone" ? "gone" : "ok")); }
  observeWindow(window: BrowserWindowLike): void {
    window.on("closed", () => this.fact("electron.window", "ok"));
    window.webContents.on("render-process-gone", () => this.fact("electron.renderer", "gone"));
    window.webContents.on("destroyed", () => this.fact("electron.renderer", "gone"));
  }
  observeUpdater(updater: AutoUpdaterLike): void { for (const event of ["checking-for-update", "update-available", "update-not-available", "update-downloaded", "error"]) updater.on(event, () => this.fact("electron.updater", event === "error" ? "error" : "ok")); }
  registerIpc(ipc: IpcMainLike, declaration: IpcDeclaration): void {
    assertSemanticName(declaration.semanticName);
    ipc.handle(declaration.channel, async (event, ...received) => {
      const started = this.now(); const { carrier, args } = stripCarrier(received);
      const work = async () => {
        try { const result = await declaration.handler(event, ...args); this.ipc(declaration.semanticName, "request", "ok", started, Boolean(carrier)); return result; }
        catch (error) { this.ipc(declaration.semanticName, "request", "error", started, Boolean(carrier)); throw error; }
      };
      return carrier ? this.options.port.withRemoteContext(carrier.traceparent, work) : work();
    });
  }
  private fact(name: string, outcome: Outcome): void { this.options.port.record(name, { direction: "main", style: "event", outcome }); }
  private ipc(name: string, style: string, outcome: Outcome, started: number, correlated: boolean): void { this.options.port.record(name, { direction: "renderer_to_main", style, outcome, duration_ms: Math.min(MAX_DURATION_MS, Math.max(0, this.now() - started)), correlated }); }
}

export function assertSemanticName(name: string): void { if (name.length > 128 || !SEMANTIC_NAME.test(name)) throw new TypeError("semantic name must match the Chill V1 contract"); }
