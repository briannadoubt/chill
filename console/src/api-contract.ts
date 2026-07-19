export const SESSION_KEYS = {
  apiBase: "chill.console.api-base",
  token: "chill.console.user-session",
  project: "chill.console.project",
  environment: "chill.console.environment",
} as const;

export type PrivateSessionBootstrap = {
  apiBase: string;
  credential: string;
  expiresAt: string;
};

export async function bootstrapPrivateSession(
  signal?: AbortSignal,
): Promise<PrivateSessionBootstrap> {
  const response = await fetch("/api/session", {
    method: "POST",
    headers: { accept: "application/json" },
    cache: "no-store",
    signal,
  });
  if (!response.ok) {
    if (response.status === 401) {
      throw new Error("Your private-site account is not authorized for this Chill workspace.");
    }
    throw new Error("Automatic sign-in is unavailable. Use an operator session below.");
  }
  const value: unknown = await response.json();
  if (!isPrivateSessionBootstrap(value)) {
    throw new Error("Automatic sign-in returned an invalid session.");
  }
  const apiBase = normalizeApiBase(value.apiBase);
  if (!value.credential.startsWith("ch_us_") || value.credential.length < 20) {
    throw new Error("Automatic sign-in returned an invalid session.");
  }
  return { ...value, apiBase };
}

function isPrivateSessionBootstrap(value: unknown): value is PrivateSessionBootstrap {
  if (value === null || Array.isArray(value) || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return typeof record.apiBase === "string"
    && typeof record.credential === "string"
    && typeof record.expiresAt === "string";
}

export function normalizeApiBase(raw: string): string {
  const value = raw.trim().replace(/\/+$/, "");
  if (!value) return "";
  if (value.startsWith("/")) return value === "/" ? "" : value;

  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error("Enter a valid http:// or https:// API address.");
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error("The API address must use http:// or https://.");
  }
  if (parsed.username || parsed.password || parsed.search || parsed.hash) {
    throw new Error("The API address cannot contain credentials, a query, or a fragment.");
  }
  return parsed.toString().replace(/\/$/, "");
}

export function apiUrl(base: string, path: string): string {
  if (!path.startsWith("/")) throw new Error("API paths must start with a slash.");
  return `${normalizeApiBase(base)}${path}`;
}

export function parseJsonObject(raw: string, label: string): Record<string, unknown> {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    throw new Error(`${label} must contain valid JSON.`);
  }
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    throw new Error(`${label} must be a JSON object.`);
  }
  return value as Record<string, unknown>;
}

export function samplingPercentage(numerator: number, denominator: number): string {
  if (!Number.isFinite(numerator) || !Number.isFinite(denominator) || denominator <= 0) {
    return "—";
  }
  const value = (numerator / denominator) * 100;
  return `${value.toLocaleString(undefined, { maximumFractionDigits: 2 })}%`;
}

export function errorMessageForStatus(status: number, fallback = "Request failed"): string {
  if (status === 401) return "Your session is invalid or expired. Sign in again.";
  if (status === 403) return "Your role does not allow this action.";
  if (status === 409) return "That configuration conflicts with existing data.";
  if (status === 413) return "The request is larger than the server allows.";
  if (status === 422) return "The server rejected one or more configuration values.";
  if (status >= 500) return "The Chill API could not complete the request. Try again.";
  return fallback;
}

export function credentialPreview(value: string): string {
  if (value.length <= 18) return value;
  return `${value.slice(0, 12)}…${value.slice(-5)}`;
}
