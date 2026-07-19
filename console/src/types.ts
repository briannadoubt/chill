export type Capability =
  | "control:read"
  | "control:write"
  | "credentials:manage"
  | "data:read"
  | "data:delete";

export type Role = "owner" | "admin" | "developer" | "analyst" | "viewer";

export interface ConsoleActor {
  id: string;
  email: string;
  display_name: string;
  role: Role;
  capabilities: Capability[];
}

export interface ConsoleOrganization {
  id: string;
  slug: string;
  name: string;
}

export interface ConsoleDataSource {
  id: string;
  name: string;
  kind: "apple" | "android" | "web" | "server" | "otlp";
  status: string;
}

export interface ConsoleSdkKey {
  id: string;
  data_source_id: string;
  name: string;
  prefix: string;
  scopes: ("ingest:otlp" | "ingest:replay")[];
  status: string;
  created_at: string;
  expires_at?: string;
  last_used_at?: string;
}

export interface ConsoleSamplingPolicy {
  version: number;
  behavior_numerator: number;
  behavior_denominator: number;
  replay_numerator: number;
  replay_denominator: number;
  salt_version: string;
}

export interface ConsolePrivacyPolicy {
  version: number;
  document: Record<string, unknown>;
}

export interface ConsoleEnvironment {
  id: string;
  slug: string;
  name: string;
  kind: "production" | "staging" | "development" | "test";
  status: string;
  retention_days: number;
  data_sources: ConsoleDataSource[];
  sdk_keys: ConsoleSdkKey[];
  sampling?: ConsoleSamplingPolicy;
  privacy?: ConsolePrivacyPolicy;
}

export interface ConsoleSchema {
  id: string;
  version: string;
  url: string;
  definition: Record<string, unknown>;
  compatibility: "exact" | "backward" | "forward" | "full";
  status: string;
}

export interface ConsoleProject {
  id: string;
  slug: string;
  name: string;
  status: string;
  environments: ConsoleEnvironment[];
  schemas: ConsoleSchema[];
}

export interface ConsoleOverview {
  actor: ConsoleActor;
  organization: ConsoleOrganization;
  projects: ConsoleProject[];
}

export interface IssuedSdkKey {
  key_id: string;
  credential: string;
}

export interface QueryResult {
  columns: string[];
  rows: unknown[][];
  stats: {
    cache_hit: boolean;
    file_count: number;
    scan_bytes: number;
    row_count: number;
    total_duration_nano: number;
  };
}

export interface SavedQuery { id: string; name: string; description: string; plan: Record<string, unknown>; visualization: "table" | "line" | "bar" | "funnel" | "retention" | "path"; updated_at: string; }
export interface Dashboard { id: string; name: string; description: string; sharing: "private" | "organization"; layout: Array<{ saved_query_id: string; width?: number }>; updated_at: string; }
export interface AnalyticsAlert { id: string; saved_query_id: string; name: string; operator: "gt" | "gte" | "lt" | "lte" | "eq"; threshold: number; schedule_minutes: number; status: string; next_evaluation_at: string; last_evaluated_at?: string; last_value?: number; last_state?: string; }
export interface AnalyticsWorkspace { saved_queries: SavedQuery[]; dashboards: Dashboard[]; alerts: AnalyticsAlert[]; }
export interface IngestDiagnostic { id: number; request_id: string; data_source_id: string; signal_kind: string; payload_format: string; record_count: number; status: string; attempt_count: number; error_code?: string; error_message?: string; metadata: Record<string, unknown>; received_at: string; }
export interface SdkDiagnostic { data_source_id: string; source_name: string; source_kind: string; source_status: string; active_keys: number; last_used_at?: string; }
export interface DebuggerSnapshot { requests: IngestDiagnostic[]; sources: SdkDiagnostic[]; }

export type ApiError = Error & { status?: number };
