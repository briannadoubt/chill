/** Narrow, payload-blind port.  The runtime adapter supplies this interface. */
export type Outcome = "ok" | "error" | "gone" | "cancelled";
export interface ChillPort {
  record(name: string, facts: Readonly<Record<string, string | number | boolean>>): void;
  /** Runtime-owned context operation; carrier metadata never grants application authority. */
  withRemoteContext<T>(traceparent: string, work: () => T): T;
  /** Produces a child propagation context without exposing runtime mutation to the app. */
  currentCarrierTraceparent(): string;
}

export interface BrowserClient {
  recordStableFact(reason: "electron_renderer_failure"): void;
}

export const noopPort: ChillPort = { record() {}, withRemoteContext: (_traceparent, work) => work(), currentCarrierTraceparent: () => "00-11111111111111111111111111111111-1111111111111111-01" };
