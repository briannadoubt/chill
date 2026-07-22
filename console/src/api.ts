import { apiUrl, errorMessageForStatus } from "./api-contract.ts";
import type {
  ApiError,
  ConsoleDataSource,
  ConsoleOverview,
  ConsoleSchema,
  IssuedSdkKey,
  QueryResult,
  AnalyticsWorkspace,
  DebuggerSnapshot,
  SavedQuery,
  Dashboard,
  AnalyticsAlert,
} from "./types";

type RequestOptions = {
  method?: "GET" | "POST" | "PATCH" | "DELETE";
  body?: unknown;
  signal?: AbortSignal;
};

export class ChillApi {
  private readonly baseUrl: string;
  private readonly userSession: string;

  constructor(baseUrl: string, userSession: string) {
    this.baseUrl = baseUrl;
    this.userSession = userSession;
  }

  overview(signal?: AbortSignal): Promise<ConsoleOverview> {
    return this.request("/v1/console/overview", { signal });
  }

  query(
    projectId: string,
    environmentId: string,
    plan: Record<string, unknown>,
  ): Promise<QueryResult> {
    return this.request("/v1/query", {
      method: "POST",
      body: { project_id: projectId, environment_id: environmentId, plan },
    });
  }

  analytics(projectId: string, environmentId: string): Promise<AnalyticsWorkspace> {
    return this.request(`/v1/console/analytics?project_id=${encodeURIComponent(projectId)}&environment_id=${encodeURIComponent(environmentId)}`);
  }

  debugger(projectId: string, environmentId: string, signal?: AbortSignal): Promise<DebuggerSnapshot> {
    return this.request(`/v1/console/debugger?project_id=${encodeURIComponent(projectId)}&environment_id=${encodeURIComponent(environmentId)}`, { signal });
  }

  createSavedQuery(body: { project_id: string; environment_id: string; name: string; description: string; plan: Record<string, unknown>; visualization: SavedQuery["visualization"] }): Promise<SavedQuery> {
    return this.request("/v1/console/saved-queries", { method: "POST", body });
  }

  createDashboard(body: { project_id: string; environment_id: string; name: string; description: string; sharing: Dashboard["sharing"]; layout: Dashboard["layout"] }): Promise<Dashboard> {
    return this.request("/v1/console/dashboards", { method: "POST", body });
  }

  createAlert(body: { project_id: string; environment_id: string; saved_query_id: string; name: string; operator: AnalyticsAlert["operator"]; threshold: number; schedule_minutes: number }): Promise<AnalyticsAlert> {
    return this.request("/v1/console/alerts", { method: "POST", body });
  }

  archiveAnalytics(kind: "saved-query" | "dashboard" | "alert", id: string): Promise<void> {
    return this.request(`/v1/console/analytics/${kind}/${encodeURIComponent(id)}`, { method: "DELETE" });
  }

  createProject(body: { slug: string; name: string }): Promise<import("./types").ConsoleProject> {
    return this.request("/v1/console/projects", { method: "POST", body });
  }

  createEnvironment(body: {
    project_id: string;
    slug: string;
    name: string;
    kind: string;
    retention_days: number;
  }): Promise<import("./types").ConsoleEnvironment> {
    return this.request("/v1/console/environments", { method: "POST", body });
  }

  updateRetention(environmentId: string, retentionDays: number): Promise<void> {
    return this.request(`/v1/console/environments/${encodeURIComponent(environmentId)}/retention`, {
      method: "PATCH",
      body: { retention_days: retentionDays },
    });
  }

  createDataSource(body: {
    project_id: string;
    environment_id: string;
    name: string;
    kind: string;
  }): Promise<ConsoleDataSource> {
    return this.request("/v1/console/data-sources", { method: "POST", body });
  }

  createSdkKey(body: {
    project_id: string;
    environment_id: string;
    data_source_id: string;
    name: string;
    scopes: string[];
    expires_at: string | null;
  }): Promise<IssuedSdkKey> {
    return this.request("/v1/console/sdk-keys", { method: "POST", body });
  }

  rotateSdkKey(keyId: string): Promise<IssuedSdkKey> {
    return this.request(`/v1/console/sdk-keys/${encodeURIComponent(keyId)}/rotate`, {
      method: "POST",
    });
  }

  revokeSdkKey(keyId: string): Promise<void> {
    return this.request(`/v1/console/sdk-keys/${encodeURIComponent(keyId)}`, {
      method: "DELETE",
    });
  }

  activateSampling(body: {
    project_id: string;
    environment_id: string;
    behavior_numerator: number;
    behavior_denominator: number;
    replay_numerator: number;
    replay_denominator: number;
    salt_version: string;
  }): Promise<void> {
    return this.request("/v1/console/sampling", { method: "POST", body });
  }

  activatePrivacy(body: {
    project_id: string;
    environment_id: string;
    document: Record<string, unknown>;
  }): Promise<void> {
    return this.request("/v1/console/privacy", { method: "POST", body });
  }

  createSchema(body: {
    project_id: string;
    version: string;
    url: string;
    definition: Record<string, unknown>;
    compatibility: string;
  }): Promise<ConsoleSchema> {
    return this.request("/v1/console/schemas", { method: "POST", body });
  }

  private async request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    let response: Response;
    try {
      response = await fetch(apiUrl(this.baseUrl, path), {
        method: options.method ?? "GET",
        signal: options.signal,
        headers: {
          accept: "application/json",
          authorization: `Bearer ${this.userSession}`,
          ...(options.body === undefined ? {} : { "content-type": "application/json" }),
        },
        body: options.body === undefined ? undefined : JSON.stringify(options.body),
        cache: "no-store",
      });
    } catch (cause) {
      if (cause instanceof DOMException && cause.name === "AbortError") throw cause;
      throw new Error("Could not reach the Chill API. Check the address and network connection.", { cause });
    }

    if (!response.ok) {
      const body = await response.text();
      const error = new Error(errorMessageForStatus(response.status, body || response.statusText)) as ApiError;
      error.status = response.status;
      throw error;
    }
    if (response.status === 204) return undefined as T;
    try {
      return (await response.json()) as T;
    } catch {
      throw new Error("The Chill API returned an unreadable response.");
    }
  }
}
