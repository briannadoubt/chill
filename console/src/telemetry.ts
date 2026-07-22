import {
  ChillBrowser,
  type RuntimeConfiguration,
} from "@chill-observability/browser";

const relayCredential = "sites-authenticated-relay";

let activeClient: ChillBrowser | null = null;
let activePolicyVersion = "";

export type ConsoleTelemetryOptions = {
  endpoint: string;
  policyVersion: string;
  storage?: Storage;
  fetch?: typeof globalThis.fetch;
};

export function createConsoleTelemetry(options: ConsoleTelemetryOptions): ChillBrowser {
  const configuration: RuntimeConfiguration = {
    endpoint: options.endpoint,
    sdkKey: relayCredential,
    policyVersion: options.policyVersion,
    consent: "unknown",
    sampleRate: 1,
    replaySampleRate: 0,
    maxBufferedRecords: 250,
    ...(options.storage ? { storage: options.storage } : {}),
    ...(options.fetch ? { fetch: options.fetch } : {}),
  };
  const client = new ChillBrowser(configuration);
  client.setConsent("granted");
  return client;
}

export function getConsoleTelemetry(policyVersion: string): ChillBrowser {
  if (activeClient && activePolicyVersion !== policyVersion) {
    const previous = activeClient;
    previous.stop();
    void previous.flush().catch(() => undefined);
    activeClient = null;
  }
  activePolicyVersion = policyVersion;
  activeClient ??= createConsoleTelemetry({
    endpoint: window.location.origin,
    policyVersion,
    storage: window.sessionStorage,
  });
  return activeClient;
}

export async function flushConsoleTelemetry(): Promise<void> {
  try {
    await activeClient?.flush();
  } catch (error) {
    // Keep diagnostics content-free: never log payloads, endpoints, or credentials.
    console.warn("Chill console telemetry flush failed", telemetryFailureCode(error));
    // The durable SDK buffer retries later; telemetry never disrupts the console.
  }
}

export function telemetryFailureCode(error: unknown): "export_http_error" | "client_encode_error" | "client_endpoint_error" | "client_transport_error" | "client_error" {
  if (!(error instanceof Error)) return "client_error";
  if (/^Chill export failed with HTTP \d{3}$/.test(error.message)) return "export_http_error";
  if (error.message === "Chill client failed during encoding") return "client_encode_error";
  if (error.message === "Chill client failed during endpoint resolution") return "client_endpoint_error";
  if (error.message === "Chill client failed during transport") return "client_transport_error";
  return "client_error";
}

export async function closeConsoleTelemetry(): Promise<void> {
  const client = activeClient;
  activeClient = null;
  activePolicyVersion = "";
  if (!client) return;
  client.stop();
  try {
    await client.flush();
  } catch {
    // A closing page cannot guarantee delivery; keep product behavior unaffected.
  }
}

const controlActions: Record<string, string> = {
  "Creating project": "console.project.create",
  "Creating environment": "console.environment.create",
  "Creating data source": "console.data_source.create",
  "Issuing SDK key": "console.sdk_key.issue",
  "Rotating SDK key": "console.sdk_key.rotate",
  "Revoking SDK key": "console.sdk_key.revoke",
  "Registering schema": "console.schema.register",
  "Updating retention": "console.retention.update",
  "Activating sampling": "console.sampling.activate",
  "Activating privacy policy": "console.privacy.activate",
};

export function controlActionName(label: string): string {
  return controlActions[label] ?? "console.control.change";
}
