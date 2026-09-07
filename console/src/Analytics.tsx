import {
  type FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { ChillApi } from "./api";
import {
  type AnalyticsKind,
  describeQueryRange,
  previousPeriodPlan,
  queryKind,
  resolveQueryPlan,
  retentionModel,
  rowsAsObjects,
  seriesValues,
} from "./analytics-model";
import type {
  AnalyticsAlert,
  AnalyticsWorkspace,
  ConsoleEnvironment,
  ConsoleProject,
  Dashboard,
  DebuggerSnapshot,
  QueryResult,
  SavedQuery,
} from "./types";

type StudioTab = "home" | "live" | "journeys" | "funnels" | "cohorts" | "replay" | "workspace";
type Visualization = SavedQuery["visualization"];
type LoadState = "idle" | "loading" | "ready" | "stale" | "error";
export type AnalyticsConnectionState = "loading" | "connected" | "stale" | "error";

type QueryExecution = {
  kind: AnalyticsKind;
  sourcePlan: Record<string, unknown>;
  evaluatedPlan: Record<string, unknown>;
  visualization: Visualization;
  result: QueryResult | null;
  state: LoadState;
  error: string;
  evaluatedAt: number;
};

type Evidence = {
  title: string;
  detail: string;
  rows?: Array<{ label: string; value: string }>;
  traceId?: string;
  sessionId?: string;
};

const tabs: Array<{ id: StudioTab; label: string }> = [
  { id: "home", label: "Product health" },
  { id: "live", label: "Live debugger" },
  { id: "journeys", label: "Journeys & traces" },
  { id: "funnels", label: "Funnels & paths" },
  { id: "cohorts", label: "Cohorts & retention" },
  { id: "replay", label: "Session replay" },
  { id: "workspace", label: "Manage" },
];

export function AnalyticsStudio({
  api,
  project,
  environment,
  canRead,
  canWrite,
  onConnectionState,
}: {
  api: ChillApi;
  project: ConsoleProject;
  environment: ConsoleEnvironment;
  canRead: boolean;
  canWrite: boolean;
  onConnectionState?: (state: AnalyticsConnectionState) => void;
}) {
  const [tab, setTab] = useState<StudioTab>("home");
  const [draftPlan, setDraftPlan] = useState<Record<string, unknown>>(() => planTemplate("aggregate"));
  const [visualization, setVisualization] = useState<Visualization>("line");
  const [execution, setExecution] = useState<QueryExecution | null>(null);
  const [workspace, setWorkspace] = useState<AnalyticsWorkspace | null>(null);
  const [workspaceState, setWorkspaceState] = useState<LoadState>("idle");
  const [workspaceError, setWorkspaceError] = useState("");
  const [debuggerData, setDebuggerData] = useState<DebuggerSnapshot | null>(null);
  const [liveExecution, setLiveExecution] = useState<QueryExecution | null>(null);
  const [pulseExecution, setPulseExecution] = useState<QueryExecution | null>(null);
  const [pulsePrevious, setPulsePrevious] = useState<QueryResult | null>(null);
  const [streamState, setStreamState] = useState<LoadState>("idle");
  const [streamError, setStreamError] = useState("");
  const [evidence, setEvidence] = useState<Evidence | null>(null);

  const loadWorkspace = useCallback(async () => {
    if (!canRead) return;
    setWorkspaceState((current) => current === "idle" ? "loading" : current);
    try {
      const next = await api.analytics(project.id, environment.id);
      setWorkspace(next);
      setWorkspaceState("ready");
      setWorkspaceError("");
    } catch (cause) {
      setWorkspaceState((current) => current === "ready" || current === "stale" ? "stale" : "error");
      setWorkspaceError(message(cause));
    }
  }, [api, canRead, environment.id, project.id]);

  useEffect(() => {
    queueMicrotask(() => void loadWorkspace());
  }, [loadWorkspace]);

  useEffect(() => {
    if ((tab !== "home" && tab !== "live") || !canRead) return;
    let active = true;
    let controller = new AbortController();
    const refresh = async () => {
      const startedAt = Date.now();
      setStreamState((current) => current === "idle" ? "loading" : current);
      const sourcePlan = tab === "live" ? planTemplate("events") : planTemplate("aggregate");
      const evaluatedPlan = resolveQueryPlan(sourcePlan, startedAt);
      try {
        const [snapshot, current, previous] = await Promise.all([
          api.debugger(project.id, environment.id, controller.signal),
          api.query(project.id, environment.id, evaluatedPlan),
          tab === "home"
            ? api.query(project.id, environment.id, previousPeriodPlan(sourcePlan, startedAt))
            : Promise.resolve(null),
        ]);
        if (!active) return;
        const next: QueryExecution = {
          kind: queryKind(sourcePlan) ?? "events",
          sourcePlan,
          evaluatedPlan,
          visualization: tab === "home" ? "line" : "table",
          result: current,
          state: "ready",
          error: "",
          evaluatedAt: startedAt,
        };
        setDebuggerData(snapshot);
        if (tab === "home") {
          setPulseExecution(next);
          setPulsePrevious(previous);
        } else {
          setLiveExecution(next);
        }
        setStreamState("ready");
        setStreamError("");
      } catch (cause) {
        if (!active || isAbort(cause)) return;
        setStreamState((current) => current === "ready" || current === "stale" ? "stale" : "error");
        setStreamError(message(cause));
      }
    };
    void refresh();
    const timer = window.setInterval(() => {
      controller.abort();
      controller = new AbortController();
      void refresh();
    }, tab === "live" ? 4_000 : 30_000);
    return () => {
      active = false;
      controller.abort();
      window.clearInterval(timer);
    };
  }, [
    api,
    canRead,
    environment.id,
    project.id,
    tab,
  ]);

  const connectionState = useMemo<AnalyticsConnectionState>(() => {
    if (!canRead) return "error";
    if (workspaceState === "error" || streamState === "error") return "error";
    if (workspaceState === "stale" || streamState === "stale") return "stale";
    if (workspaceState === "loading" || streamState === "loading" || workspaceState === "idle") {
      return "loading";
    }
    return "connected";
  }, [canRead, streamState, workspaceState]);

  useEffect(() => {
    onConnectionState?.(connectionState);
  }, [connectionState, onConnectionState]);

  const execute = useCallback(async (
    nextPlan = draftPlan,
    nextVisualization = visualization,
  ) => {
    const kind = queryKind(nextPlan);
    if (!kind) return;
    const evaluatedAt = Date.now();
    const evaluatedPlan = resolveQueryPlan(nextPlan, evaluatedAt);
    setDraftPlan(nextPlan);
    setVisualization(nextVisualization);
    setExecution((current) => ({
      kind,
      sourcePlan: nextPlan,
      evaluatedPlan,
      visualization: nextVisualization,
      result: current?.kind === kind ? current.result : null,
      state: "loading",
      error: "",
      evaluatedAt,
    }));
    try {
      const result = await api.query(project.id, environment.id, evaluatedPlan);
      setExecution({
        kind,
        sourcePlan: nextPlan,
        evaluatedPlan,
        visualization: nextVisualization,
        result,
        state: "ready",
        error: "",
        evaluatedAt,
      });
    } catch (cause) {
      setExecution((current) => ({
        kind,
        sourcePlan: nextPlan,
        evaluatedPlan,
        visualization: nextVisualization,
        result: current?.kind === kind ? current.result : null,
        state: current?.kind === kind && current.result ? "stale" : "error",
        error: message(cause),
        evaluatedAt,
      }));
    }
  }, [api, draftPlan, environment.id, project.id, visualization]);

  const changeTab = (next: StudioTab) => {
    setTab(next);
    setExecution(null);
    setEvidence(null);
  };

  const openPlan = (nextPlan: Record<string, unknown>, nextVisualization: Visualization) => {
    const nextTab = tabForKind(queryKind(nextPlan));
    setTab(nextTab);
    setDraftPlan(nextPlan);
    setVisualization(nextVisualization);
    setEvidence(null);
    void execute(nextPlan, nextVisualization);
  };

  const openTrace = (traceId: string) => {
    openPlan(planTemplate("trace", { traceId }), "table");
  };
  const openReplay = (sessionId: string) => {
    openPlan(planTemplate("replay", { sessionId }), "table");
  };

  const activeError = execution?.error || streamError || workspaceError;
  const resultFor = (...kinds: AnalyticsKind[]): QueryExecution | null =>
    execution && kinds.includes(execution.kind) ? execution : null;

  return (
    <div className="page analytics-page">
      <nav className="studio-tabs" aria-label="Analytics tools" role="tablist">
        {tabs.map((item) => (
          <button
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            key={item.id}
            className={tab === item.id ? "active" : ""}
            onClick={() => changeTab(item.id)}
          >
            {item.label}
          </button>
        ))}
      </nav>

      {!canRead && (
        <div className="analytics-callout" role="alert">
          Your role needs <code>data:read</code> to use analytics.
        </div>
      )}
      {activeError && (
        <div className={`notice error analytics-error ${connectionState === "stale" ? "stale" : ""}`} role="alert">
          <strong>{connectionState === "stale" ? "Showing last successful data." : "Analytics is unavailable."}</strong>
          <span>{activeError}</span>
        </div>
      )}
      <div className="sr-only" aria-live="polite">
        {connectionState === "loading" ? "Analytics is loading." : ""}
        {connectionState === "stale" ? "Analytics data is stale." : ""}
        {execution?.state === "ready" ? "Analysis complete." : ""}
      </div>

      {tab === "home" && (
        <PulseHome
          project={project}
          environment={environment}
          workspace={workspace}
          workspaceState={workspaceState}
          snapshot={debuggerData}
          pulse={pulseExecution}
          previous={pulsePrevious}
          streamState={streamState}
          onNavigate={changeTab}
          onEvidence={setEvidence}
          onTrace={openTrace}
          onReplay={openReplay}
          api={api}
        />
      )}
      {tab === "live" && (
        <LiveDebugger
          execution={liveExecution}
          snapshot={debuggerData}
          state={streamState}
          onTrace={openTrace}
          onReplay={openReplay}
        />
      )}
      {tab === "journeys" && (
        <JourneyBuilder
          key={planSignature(draftPlan)}
          execution={resultFor("events", "trace")}
          initialPlan={draftPlan}
          execute={execute}
          onTrace={openTrace}
          onReplay={openReplay}
        />
      )}
      {tab === "funnels" && (
        <FunnelBuilder
          key={planSignature(draftPlan)}
          execution={resultFor("funnel", "path")}
          initialPlan={draftPlan}
          execute={execute}
          onTrace={openTrace}
          onReplay={openReplay}
          clearResult={() => setExecution(null)}
        />
      )}
      {tab === "cohorts" && (
        <CohortBuilder
          key={planSignature(draftPlan)}
          execution={resultFor("cohort", "retention")}
          initialPlan={draftPlan}
          execute={execute}
          clearResult={() => setExecution(null)}
        />
      )}
      {tab === "replay" && (
        <ReplayBuilder
          key={planSignature(draftPlan)}
          execution={resultFor("replay")}
          initialPlan={draftPlan}
          execute={execute}
        />
      )}
      {tab === "workspace" && (
        <WorkspacePanel
          api={api}
          project={project}
          environment={environment}
          workspace={workspace}
          workspaceState={workspaceState}
          currentExecution={execution}
          canWrite={canWrite}
          reload={loadWorkspace}
          openQuery={openPlan}
          onEvidence={setEvidence}
          onTrace={openTrace}
          onReplay={openReplay}
        />
      )}
      {evidence && (
        <EvidenceDrawer
          evidence={evidence}
          onClose={() => setEvidence(null)}
          onTrace={openTrace}
          onReplay={openReplay}
        />
      )}
    </div>
  );
}

function PulseHome({
  project,
  environment,
  workspace,
  workspaceState,
  snapshot,
  pulse,
  previous,
  streamState,
  onNavigate,
  onEvidence,
  onTrace,
  onReplay,
  api,
}: {
  project: ConsoleProject;
  environment: ConsoleEnvironment;
  workspace: AnalyticsWorkspace | null;
  workspaceState: LoadState;
  snapshot: DebuggerSnapshot | null;
  pulse: QueryExecution | null;
  previous: QueryResult | null;
  streamState: LoadState;
  onNavigate: (tab: StudioTab) => void;
  onEvidence: (evidence: Evidence) => void;
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
  api: ChillApi;
}) {
  const requests = snapshot?.requests ?? [];
  const sources = snapshot?.sources ?? [];
  const failed = requests.filter((request) => request.status === "dead_letter");
  const triggered = workspace?.alerts.filter((alert) => alert.last_state === "triggered") ?? [];
  const totalEvents = sumMetric(pulse?.result ?? null);
  const activeSources = sources.filter((source) => source.source_status === "active").length;
  const mostRecent = latestTimestamp(requests.map((request) => request.received_at));
  const isStale = streamState === "stale" || workspaceState === "stale";
  const headline = failed.length || triggered.length
    ? `${project.name} needs attention.`
    : totalEvents > 0
      ? `${project.name} is reporting normally.`
      : `Set up ${project.name}'s product pulse.`;
  const detail = failed.length
    ? `${failed.length} ingestion ${failed.length === 1 ? "request needs" : "requests need"} investigation.`
    : triggered.length
      ? `${triggered.length} ${triggered.length === 1 ? "alert is" : "alerts are"} outside the configured threshold.`
      : totalEvents > 0
        ? `${formatCompact(totalEvents)} behavior records were observed in the rolling seven-day window.`
        : "Connect telemetry or save an analysis to make this overview answer product-health questions.";
  const dashboard = workspace?.dashboards[0];

  return (
    <div className="pulse-home">
      <header className="pulse-hero">
        <div>
          <span className="eyebrow">Product health</span>
          <h1>{headline}</h1>
          <p>{detail}</p>
        </div>
        <div className="pulse-actions">
          <button type="button" className="button primary" onClick={() => onNavigate(failed.length ? "live" : "funnels")}>
            Investigate change
          </button>
          <button
            type="button"
            className="button secondary"
            onClick={() => onEvidence({
              title: "Current vs previous period",
              detail: comparisonDetail(pulse?.result ?? null, previous),
              rows: [
                { label: "Current range", value: pulse ? describeQueryRange(pulse.sourcePlan, pulse.evaluatedAt) : "Loading" },
                { label: "Comparison", value: pulse?.result ? comparison(pulse.result, previous) : "Loading" },
              ],
            })}
          >
            Compare period
          </button>
          <button type="button" className="button secondary" onClick={() => onNavigate("workspace")}>
            Edit dashboard
          </button>
        </div>
      </header>

      <div className="pulse-scope" aria-label="Analytics scope and freshness">
        <div><span>Project</span><strong>{project.name}</strong></div>
        <div><span>Environment</span><strong className="environment-value">{environment.name}</strong></div>
        <div><span>Time range</span><strong>Last 7 days</strong></div>
        <div><span>Timezone</span><strong>UTC</strong></div>
        <div>
          <span>Freshness</span>
          <strong className={isStale ? "freshness-stale" : "freshness-live"}>
            {isStale ? "Stale" : mostRecent ? `Updated ${relativeTime(mostRecent)}` : "Waiting for data"}
          </strong>
        </div>
      </div>

      {(streamState === "loading" || workspaceState === "loading") && !pulse?.result ? (
        <AnalyticsLoading label="Building the product-health overview…" />
      ) : (
        <section className="pulse-metrics" aria-label="Product health metrics">
          <PulseMetric
            label="Behavior records"
            value={formatCompact(totalEvents)}
            change={pulse?.result ? comparison(pulse.result, previous) : "No comparison yet"}
            tone="good"
            onOpen={() => onEvidence({
              title: "Behavior records",
              detail: "Count of canonical behavior records in the evaluated rolling range.",
              rows: [
                { label: "Current", value: totalEvents.toLocaleString() },
                { label: "Evaluated range", value: pulse ? describeQueryRange(pulse.sourcePlan, pulse.evaluatedAt) : "Unavailable" },
              ],
            })}
          />
          <PulseMetric
            label="Reporting sources"
            value={sources.length ? `${activeSources}/${sources.length}` : "0"}
            change={sources.length ? `${activeSources} active` : "No sources reporting"}
            tone={activeSources === sources.length && sources.length ? "good" : "neutral"}
            onOpen={() => onEvidence({
              title: "Reporting sources",
              detail: "Source status comes from the Rust control plane and latest SDK-key usage.",
              rows: sources.map((source) => ({
                label: source.source_name,
                value: `${source.source_status} · last used ${relativeTime(source.last_used_at)}`,
              })),
            })}
          />
          <PulseMetric
            label="Failed ingestion"
            value={formatCompact(failed.length)}
            change={failed.length ? "Needs investigation" : "No dead-letter requests"}
            tone={failed.length ? "danger" : "good"}
            onOpen={() => onEvidence({
              title: "Failed ingestion requests",
              detail: failed.length ? "These requests exhausted processing and are not included in live analytics." : "No dead-letter requests are visible in the latest diagnostic window.",
              rows: failed.slice(0, 12).map((request) => ({
                label: request.request_id,
                value: `${request.signal_kind} · ${request.error_code ?? "unknown error"}`,
              })),
            })}
          />
          <PulseMetric
            label="Alerts needing attention"
            value={formatCompact(triggered.length)}
            change={triggered.length ? "Threshold exceeded" : `${workspace?.alerts.length ?? 0} configured`}
            tone={triggered.length ? "danger" : "good"}
            onOpen={() => onEvidence({
              title: "Alert status",
              detail: "Latest persisted alert evaluations for this project and environment.",
              rows: (workspace?.alerts ?? []).map((alert) => ({
                label: alert.name,
                value: alertSummary(alert),
              })),
            })}
          />
        </section>
      )}

      <div className="pulse-layout">
        <section className="analytics-surface pulse-trend">
          <div className="surface-heading">
            <div>
              <span className="eyebrow">Volume over time</span>
              <h2>Behavior activity</h2>
            </div>
            <span className="result-count">Daily · UTC</span>
          </div>
          <SeriesChart result={pulse?.result ?? null} line label="Daily behavior activity" />
          {pulse && <p className="range-caption">{describeQueryRange(pulse.sourcePlan, pulse.evaluatedAt)}</p>}
        </section>
        <aside className="pulse-attention">
          <section className="analytics-surface">
            <div className="surface-heading">
              <div><span className="eyebrow">Needs attention</span><h2>Evidence</h2></div>
            </div>
            {failed.length ? (
              failed.slice(0, 3).map((request) => (
                <button
                  type="button"
                  className="attention-card danger"
                  key={request.id}
                  onClick={() => onEvidence({
                    title: `${request.signal_kind} ingestion failed`,
                    detail: request.error_message ?? "The request entered the dead-letter state.",
                    rows: [
                      { label: "Request", value: request.request_id },
                      { label: "Records", value: request.record_count.toLocaleString() },
                      { label: "Attempts", value: request.attempt_count.toLocaleString() },
                      { label: "Received", value: request.received_at },
                    ],
                  })}
                >
                  <strong>{request.error_code ?? "Ingestion failure"}</strong>
                  <span>{request.record_count} records · {relativeTime(request.received_at)}</span>
                </button>
              ))
            ) : triggered.length ? (
              triggered.slice(0, 3).map((alert) => (
                <button
                  type="button"
                  className="attention-card danger"
                  key={alert.id}
                  onClick={() => onEvidence({
                    title: alert.name,
                    detail: alertSummary(alert),
                  })}
                >
                  <strong>{alert.name}</strong>
                  <span>{alertSummary(alert)}</span>
                </button>
              ))
            ) : (
              <div className="healthy-state"><strong>No active issues</strong><span>Latest diagnostics and alerts are healthy.</span></div>
            )}
          </section>
          <section className="analytics-surface freshness-summary">
            <div><strong>Data freshness</strong><span className={isStale ? "freshness-stale" : "freshness-live"}>{isStale ? "Stale" : "Live"}</span></div>
            <dl>
              <div><dt>Most recent request</dt><dd>{relativeTime(mostRecent)}</dd></div>
              <div><dt>Active sources</dt><dd>{activeSources} of {sources.length}</dd></div>
            </dl>
            <button type="button" className="text-action" onClick={() => onNavigate("live")}>View source diagnostics</button>
          </section>
        </aside>
      </div>

      <section className="analytics-surface pulse-dashboard">
        <div className="surface-heading">
          <div>
            <span className="eyebrow">Team dashboard</span>
            <h2>{dashboard?.name ?? "No shared dashboard yet"}</h2>
          </div>
          <button type="button" className="text-action" onClick={() => onNavigate("workspace")}>
            {dashboard ? "Manage dashboard" : "Create dashboard"}
          </button>
        </div>
        {dashboard && workspace ? (
          <DashboardPreview
            api={api}
            project={project}
            environment={environment}
            dashboard={dashboard}
            queries={workspace.saved_queries}
            onEvidence={onEvidence}
            onTrace={onTrace}
            onReplay={onReplay}
          />
        ) : (
          <AnalyticsEmpty
            title="Create the first shared dashboard"
            detail="Save a useful analysis, then choose the panels your team should monitor."
            action={<button type="button" className="button secondary" onClick={() => onNavigate("workspace")}>Open dashboard setup</button>}
          />
        )}
      </section>
    </div>
  );
}

function PulseMetric({
  label,
  value,
  change,
  tone,
  onOpen,
}: {
  label: string;
  value: string;
  change: string;
  tone: "good" | "danger" | "neutral";
  onOpen: () => void;
}) {
  return (
    <button type="button" className={`pulse-metric ${tone}`} onClick={onOpen}>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{change}</small>
      <span className="metric-affordance">View evidence</span>
    </button>
  );
}

function LiveDebugger({
  execution,
  snapshot,
  state,
  onTrace,
  onReplay,
}: {
  execution: QueryExecution | null;
  snapshot: DebuggerSnapshot | null;
  state: LoadState;
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
}) {
  const records = rowsAsObjects(execution?.result ?? null);
  return (
    <>
      <AnalyticsHeading
        eyebrow="Live behavior"
        title="See what is arriving now."
        detail="The stream refreshes every four seconds. If the connection drops, Chill keeps the last result visible and labels it stale."
        freshness={state === "stale" ? "Stale" : state === "ready" ? "Live · 4s" : "Connecting"}
      />
      <div className="analytics-grid live-grid">
        <section className="analytics-surface live-feed">
          <div className="surface-heading">
            <div><span className="eyebrow">Recent stream</span><h2>Behavior timeline</h2></div>
            <span className="result-count">{records.length} records</span>
          </div>
          {state === "loading" && !records.length ? (
            <AnalyticsLoading label="Connecting to the behavior stream…" />
          ) : (
            <div className="telemetry-list">
              {records.length
                ? records.map((record, index) => (
                  <TelemetryRecord
                    key={String(record.record_id ?? index)}
                    record={record}
                    onTrace={onTrace}
                    onReplay={onReplay}
                  />
                ))
                : <AnalyticsEmpty title="Waiting for telemetry" detail="Send an event from an SDK and it will appear here automatically." />}
            </div>
          )}
        </section>
        <aside className="analytics-rail">
          <section className="analytics-surface">
            <div className="surface-heading"><div><span className="eyebrow">SDK health</span><h2>Sources</h2></div></div>
            <div className="health-list">
              {snapshot?.sources.length
                ? snapshot.sources.map((source) => (
                  <div key={source.data_source_id}>
                    <span className={`health-dot ${source.source_status}`} aria-hidden="true" />
                    <div>
                      <strong>{source.source_name}</strong>
                      <span>{source.source_kind} · {source.active_keys} active key{source.active_keys === 1 ? "" : "s"}</span>
                    </div>
                    <time>{relativeTime(source.last_used_at)}</time>
                  </div>
                ))
                : <AnalyticsEmpty title="No reporting sources" detail="Register a source and use an active SDK key to begin." />}
            </div>
          </section>
          <section className="analytics-surface">
            <div className="surface-heading"><div><span className="eyebrow">Ingest pipeline</span><h2>Requests & rejections</h2></div></div>
            <div className="request-list">
              {snapshot?.requests.length
                ? snapshot.requests.slice(0, 12).map((request) => (
                  <details key={request.id}>
                    <summary>
                      <span className={`pipeline-state ${request.status}`} aria-hidden="true" />
                      <strong>{request.signal_kind}</strong>
                      <span>{request.record_count} records</span>
                      <time>{relativeTime(request.received_at)}</time>
                    </summary>
                    <dl>
                      <div><dt>Request</dt><dd>{request.request_id}</dd></div>
                      <div><dt>Format</dt><dd>{request.payload_format}</dd></div>
                      <div><dt>Attempts</dt><dd>{request.attempt_count}</dd></div>
                      {request.error_code && <div><dt>Rejected</dt><dd>{request.error_code}: {request.error_message}</dd></div>}
                    </dl>
                    <pre>{JSON.stringify(request.metadata, null, 2)}</pre>
                  </details>
                ))
                : <AnalyticsEmpty title="No ingestion requests" detail="Requests will appear as soon as telemetry reaches this environment." />}
            </div>
          </section>
        </aside>
      </div>
    </>
  );
}

type BuilderProps = {
  execution: QueryExecution | null;
  initialPlan: Record<string, unknown>;
  execute: (plan: Record<string, unknown>, visualization: Visualization) => Promise<void>;
};

function JourneyBuilder({
  execution,
  initialPlan,
  execute,
  onTrace,
  onReplay,
}: BuilderProps & {
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
}) {
  const initialKind = queryKind(initialPlan);
  const [mode, setMode] = useState<"session" | "trace">(initialKind === "trace" ? "trace" : "session");
  const [identifier, setIdentifier] = useState(() => initialIdentifier(initialPlan));
  const submit = (event: FormEvent) => {
    event.preventDefault();
    void execute(
      mode === "trace"
        ? planTemplate("trace", { traceId: identifier })
        : planTemplate("events", { sessionId: identifier }),
      "table",
    );
  };
  return (
    <>
      <AnalyticsHeading
        eyebrow="Journey investigation"
        title="Follow a symptom across UI and services."
        detail="A session timeline preserves page ancestry and links every traced action to its distributed work."
        freshness="On demand"
      />
      <BuilderLayout controls={(
        <form className="inline-builder" onSubmit={submit}>
          <label><span className="sr-only">Identifier type</span><select value={mode} onChange={(event) => setMode(event.target.value as "session" | "trace")}><option value="session">Session ID</option><option value="trace">Trace ID</option></select></label>
          <label><span className="sr-only">{mode === "trace" ? "Trace ID" : "Session ID"}</span><input value={identifier} onChange={(event) => setIdentifier(event.target.value.trim())} placeholder={mode === "trace" ? "32-character trace ID" : "Session ID"} required /></label>
          <button type="submit" disabled={execution?.state === "loading"}>{execution?.state === "loading" ? "Tracing…" : "Explore"}</button>
        </form>
      )}>
        <ResultMeta execution={execution} />
        <div className="journey-timeline">
          {rowsAsObjects(execution?.result ?? null).map((record, index) => (
            <TelemetryRecord key={index} record={record} onTrace={onTrace} onReplay={onReplay} expanded />
          ))}
        </div>
        {execution?.result && execution.result.rows.length === 0 && <AnalyticsEmpty title="No journey found" detail="Check the identifier and selected environment." />}
      </BuilderLayout>
    </>
  );
}

function FunnelBuilder({
  execution,
  initialPlan,
  execute,
  onTrace,
  onReplay,
  clearResult,
}: BuilderProps & {
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
  clearResult: () => void;
}) {
  const initialKind = queryKind(initialPlan);
  const [analysis, setAnalysis] = useState<"funnel" | "path">(initialKind === "path" ? "path" : "funnel");
  const [steps, setSteps] = useState(() => initialFunnelSteps(initialPlan));
  const [mode, setMode] = useState("ordered");
  const [windowMinutes, setWindowMinutes] = useState(60);
  const [breakdown, setBreakdown] = useState("none");
  const [exclusion, setExclusion] = useState("");
  const [propertyKey, setPropertyKey] = useState("");
  const [propertyValue, setPropertyValue] = useState("");
  const [anchor, setAnchor] = useState(() => initialPathAnchor(initialPlan));
  const [direction, setDirection] = useState("after");
  const switchAnalysis = (next: "funnel" | "path") => {
    setAnalysis(next);
    clearResult();
  };
  const run = (event: FormEvent) => {
    event.preventDefault();
    if (analysis === "path") {
      void execute(planTemplate("path", { anchor, direction }), "path");
      return;
    }
    const names = steps.split(",").map((item) => item.trim()).filter(Boolean).slice(0, 8);
    void execute(
      planTemplate("funnel", { names, mode, windowMinutes, breakdown, exclusion, propertyKey, propertyValue }),
      "funnel",
    );
  };
  return (
    <>
      <AnalyticsHeading
        eyebrow="Conversion analysis"
        title="Find conversion and the paths around it."
        detail="Ordered or unordered funnels and path transitions are compiled by Rust against one immutable manifest."
        freshness="On demand"
      />
      <BuilderLayout controls={(
        <form className="builder-form" onSubmit={run}>
          <div className="segmented" aria-label="Analysis type">
            <button type="button" aria-pressed={analysis === "funnel"} className={analysis === "funnel" ? "active" : ""} onClick={() => switchAnalysis("funnel")}>Funnel</button>
            <button type="button" aria-pressed={analysis === "path"} className={analysis === "path" ? "active" : ""} onClick={() => switchAnalysis("path")}>Path discovery</button>
          </div>
          {analysis === "funnel" ? (
            <>
              <label><span>Semantic steps · comma separated</span><input value={steps} onChange={(event) => setSteps(event.target.value)} required /></label>
              <div className="field-row">
                <label><span>Semantics</span><select value={mode} onChange={(event) => setMode(event.target.value)}><option value="ordered">Ordered</option><option value="unordered">Unordered</option></select></label>
                <label><span>Break down first step</span><select value={breakdown} onChange={(event) => setBreakdown(event.target.value)}><option value="none">No breakdown</option><option value="platform">Platform</option><option value="page">Page</option><option value="kind">Behavior kind</option><option value="name">Event name</option></select></label>
              </div>
              <div className="field-row">
                <label><span>Conversion window</span><div className="input-unit"><input type="number" min={1} max={44640} value={windowMinutes} onChange={(event) => setWindowMinutes(Number(event.target.value))} /><span>min</span></div></label>
                <label><span>Exclude subjects who did</span><input value={exclusion} onChange={(event) => setExclusion(event.target.value.trim())} placeholder="subscription.cancelled" /></label>
              </div>
              <div className="field-row">
                <label><span>Annotation property</span><input value={propertyKey} onChange={(event) => setPropertyKey(event.target.value.trim())} placeholder="account.tier" /></label>
                <label><span>Exact value</span><input value={propertyValue} onChange={(event) => setPropertyValue(event.target.value)} placeholder="internal" /></label>
              </div>
            </>
          ) : (
            <div className="field-row">
              <label><span>Anchor event</span><input value={anchor} onChange={(event) => setAnchor(event.target.value)} required /></label>
              <label><span>Direction</span><select value={direction} onChange={(event) => setDirection(event.target.value)}><option value="after">What happens after</option><option value="before">What happens before</option></select></label>
            </div>
          )}
          <button className="run-analysis" type="submit" disabled={execution?.state === "loading"}>{execution?.state === "loading" ? "Calculating…" : "Run analysis"}</button>
        </form>
      )}>
        <ResultMeta execution={execution} />
        {analysis === "funnel"
          ? <FunnelChart result={execution?.kind === "funnel" ? execution.result : null} onTrace={onTrace} onReplay={onReplay} />
          : <PathChart result={execution?.kind === "path" ? execution.result : null} onTrace={onTrace} />}
      </BuilderLayout>
    </>
  );
}

function CohortBuilder({
  execution,
  initialPlan,
  execute,
  clearResult,
}: BuilderProps & { clearResult: () => void }) {
  const initialKind = queryKind(initialPlan);
  const [analysis, setAnalysis] = useState<"cohort" | "retention">(initialKind === "cohort" ? "cohort" : "retention");
  const [predicate, setPredicate] = useState<"frequency" | "sequence">("frequency");
  const [start, setStart] = useState(() => initialRetentionStart(initialPlan));
  const [returnEvent, setReturnEvent] = useState(() => initialRetentionReturn(initialPlan));
  const [minimum, setMinimum] = useState(2);
  const [sequence, setSequence] = useState("trial.started, feature.used, subscription.started");
  const [windowMinutes, setWindowMinutes] = useState(1440);
  const [propertyKey, setPropertyKey] = useState("");
  const [propertyValue, setPropertyValue] = useState("");
  const switchAnalysis = (next: "cohort" | "retention") => {
    setAnalysis(next);
    clearResult();
  };
  const run = (event: FormEvent) => {
    event.preventDefault();
    const sequenceNames = sequence.split(",").map((item) => item.trim()).filter(Boolean).slice(0, 8);
    void execute(
      analysis === "retention"
        ? planTemplate("retention", { start, returnEvent })
        : planTemplate("cohort", {
          name: returnEvent,
          minimum,
          sequence: predicate === "sequence" ? sequenceNames : [],
          windowMinutes,
          propertyKey,
          propertyValue,
        }),
      analysis === "retention" ? "retention" : "table",
    );
  };
  return (
    <>
      <AnalyticsHeading
        eyebrow="Audience analysis"
        title="Define behavior once, reuse the audience."
        detail="Event, property, sequence, frequency, and time-window predicates explain membership; retention measures recurrence."
        freshness="On demand"
      />
      <BuilderLayout controls={(
        <form className="builder-form" onSubmit={run}>
          <div className="segmented" aria-label="Analysis type">
            <button type="button" aria-pressed={analysis === "retention"} className={analysis === "retention" ? "active" : ""} onClick={() => switchAnalysis("retention")}>Retention</button>
            <button type="button" aria-pressed={analysis === "cohort"} className={analysis === "cohort" ? "active" : ""} onClick={() => switchAnalysis("cohort")}>Behavioral cohort</button>
          </div>
          {analysis === "retention" && <label><span>Cohort-forming event</span><input value={start} onChange={(event) => setStart(event.target.value)} required /></label>}
          {analysis === "cohort" && (
            <div className="segmented" aria-label="Cohort predicate">
              <button type="button" aria-pressed={predicate === "frequency"} className={predicate === "frequency" ? "active" : ""} onClick={() => setPredicate("frequency")}>Frequency</button>
              <button type="button" aria-pressed={predicate === "sequence"} className={predicate === "sequence" ? "active" : ""} onClick={() => setPredicate("sequence")}>Sequence</button>
            </div>
          )}
          <label>
            <span>{analysis === "retention" ? "Return event" : predicate === "sequence" ? "Ordered events · comma separated" : "Matching event"}</span>
            <input
              value={analysis === "cohort" && predicate === "sequence" ? sequence : returnEvent}
              onChange={(event) => analysis === "cohort" && predicate === "sequence" ? setSequence(event.target.value) : setReturnEvent(event.target.value)}
              required
            />
          </label>
          {analysis === "cohort" && predicate === "frequency" && <label><span>Minimum occurrences</span><input type="number" min={1} value={minimum} onChange={(event) => setMinimum(Number(event.target.value))} /></label>}
          {analysis === "cohort" && predicate === "sequence" && <label><span>Sequence window</span><div className="input-unit"><input type="number" min={1} max={44640} value={windowMinutes} onChange={(event) => setWindowMinutes(Number(event.target.value))} /><span>min</span></div></label>}
          {analysis === "cohort" && (
            <div className="field-row">
              <label><span>Annotation property</span><input value={propertyKey} onChange={(event) => setPropertyKey(event.target.value.trim())} placeholder="account.tier" /></label>
              <label><span>Exact value</span><input value={propertyValue} onChange={(event) => setPropertyValue(event.target.value)} placeholder="enterprise" /></label>
            </div>
          )}
          <button className="run-analysis" type="submit" disabled={execution?.state === "loading"}>{execution?.state === "loading" ? "Calculating…" : "Run analysis"}</button>
        </form>
      )}>
        <ResultMeta execution={execution} />
        {analysis === "retention"
          ? <RetentionMatrix result={execution?.kind === "retention" ? execution.result : null} />
          : <DataTable result={execution?.kind === "cohort" ? execution.result : null} />}
      </BuilderLayout>
    </>
  );
}

function ReplayBuilder({ execution, initialPlan, execute }: BuilderProps) {
  const replay = objectValue(initialPlan.replay);
  const [identifier, setIdentifier] = useState(() => initialIdentifier(initialPlan));
  const [kind, setKind] = useState<"session" | "replay">(stringValue(replay?.replay_id) ? "replay" : "session");
  const [cursor, setCursor] = useState(0);
  const records = rowsAsObjects(execution?.result ?? null);
  const selected = records[Math.min(cursor, Math.max(0, records.length - 1))];
  const run = (event: FormEvent) => {
    event.preventDefault();
    setCursor(0);
    void execute(
      planTemplate("replay", kind === "session" ? { sessionId: identifier } : { replayId: identifier }),
      "table",
    );
  };
  return (
    <>
      <AnalyticsHeading
        eyebrow="Privacy-preserving replay"
        title="Replay structure without exposing content."
        detail="Gestures, navigation, behavior, and trace markers share one monotonic timeline. Gaps stay explicit."
        freshness="On demand"
      />
      <BuilderLayout controls={(
        <form className="inline-builder" onSubmit={run}>
          <label><span className="sr-only">Identifier type</span><select value={kind} onChange={(event) => setKind(event.target.value as "session" | "replay")}><option value="session">Session ID</option><option value="replay">Replay ID</option></select></label>
          <label><span className="sr-only">{kind === "session" ? "Session ID" : "Replay ID"}</span><input value={identifier} onChange={(event) => setIdentifier(event.target.value.trim())} placeholder="Paste identifier" required /></label>
          <button type="submit" disabled={execution?.state === "loading"}>{execution?.state === "loading" ? "Loading…" : "Load replay"}</button>
        </form>
      )}>
        <ResultMeta execution={execution} />
        <div className="replay-player">
          <div className="replay-stage">
            <div className="device-frame">
              <div className="device-status"><span>source masked</span><span>private</span></div>
              <StructuralFrame record={selected} />
            </div>
          </div>
          <aside>
            <h3>Timeline</h3>
            <input aria-label="Replay position" type="range" min={0} max={Math.max(0, records.length - 1)} value={Math.min(cursor, Math.max(0, records.length - 1))} onChange={(event) => setCursor(Number(event.target.value))} />
            <div className="replay-markers">
              {records.map((record, index) => (
                <button type="button" aria-pressed={cursor === index} className={cursor === index ? "active" : ""} key={index} onClick={() => setCursor(index)}>
                  <span>{index + 1}</span>
                  <div><strong>{replayType(record)}</strong><small>{formatNano(record.effective_occurred_at_unix_nano)}</small></div>
                </button>
              ))}
            </div>
          </aside>
        </div>
      </BuilderLayout>
    </>
  );
}

function WorkspacePanel({
  api,
  project,
  environment,
  workspace,
  workspaceState,
  currentExecution,
  canWrite,
  reload,
  openQuery,
  onEvidence,
  onTrace,
  onReplay,
}: {
  api: ChillApi;
  project: ConsoleProject;
  environment: ConsoleEnvironment;
  workspace: AnalyticsWorkspace | null;
  workspaceState: LoadState;
  currentExecution: QueryExecution | null;
  canWrite: boolean;
  reload: () => Promise<void>;
  openQuery: (plan: Record<string, unknown>, visualization: Visualization) => void;
  onEvidence: (evidence: Evidence) => void;
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
}) {
  const [queryName, setQueryName] = useState("");
  const [dashboardName, setDashboardName] = useState("");
  const [alertName, setAlertName] = useState("");
  const [selectedQuery, setSelectedQuery] = useState("");
  const [dashboardQueries, setDashboardQueries] = useState<string[]>([]);
  const [wideQueries, setWideQueries] = useState<string[]>([]);
  const [operator, setOperator] = useState<AnalyticsAlert["operator"]>("gt");
  const [threshold, setThreshold] = useState("0");
  const [schedule, setSchedule] = useState("15");
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<{ kind: "success" | "error"; text: string } | null>(null);
  const [editingQuery, setEditingQuery] = useState<SavedQuery | null>(null);
  const [editingDashboard, setEditingDashboard] = useState<Dashboard | null>(null);

  const effectiveSelectedQuery = selectedQuery || workspace?.saved_queries[0]?.id || "";

  const mutate = async (success: string, operation: () => Promise<unknown>) => {
    setBusy(true);
    setFeedback(null);
    try {
      await operation();
      await reload();
      setFeedback({ kind: "success", text: success });
    } catch (cause) {
      setFeedback({ kind: "error", text: message(cause) });
    } finally {
      setBusy(false);
    }
  };
  const saveQuery = (event: FormEvent) => {
    event.preventDefault();
    if (!currentExecution?.result || currentExecution.state !== "ready") return;
    void mutate("Saved query created.", async () => {
      const saved = await api.createSavedQuery({
        project_id: project.id,
        environment_id: environment.id,
        name: queryName,
        description: "Saved from Analytics Studio",
        plan: currentExecution.sourcePlan,
        visualization: currentExecution.visualization,
      });
      setSelectedQuery(saved.id);
      setQueryName("");
    });
  };
  const createDashboard = (event: FormEvent) => {
    event.preventDefault();
    void mutate("Dashboard created.", async () => {
      await api.createDashboard({
        project_id: project.id,
        environment_id: environment.id,
        name: dashboardName,
        description: "Shared product and reliability signals",
        sharing: "organization",
        layout: dashboardQueries.map((saved_query_id) => ({
          saved_query_id,
          width: wideQueries.includes(saved_query_id) ? 2 : 1,
        })),
      });
      setDashboardName("");
      setDashboardQueries([]);
      setWideQueries([]);
    });
  };
  const createAlert = (event: FormEvent) => {
    event.preventDefault();
    if (!effectiveSelectedQuery) return;
    void mutate("Alert created.", async () => {
      await api.createAlert({
        project_id: project.id,
        environment_id: environment.id,
        saved_query_id: effectiveSelectedQuery,
        name: alertName,
        operator,
        threshold: Number(threshold),
        schedule_minutes: Number(schedule),
      });
      setAlertName("");
    });
  };
  const archive = (kind: "saved-query" | "dashboard" | "alert", id: string, label: string) => {
    if (!window.confirm(`Archive “${label}”? It will disappear from this workspace.`)) return;
    void mutate(`${label} archived.`, () => api.archiveAnalytics(kind, id));
  };
  const togglePanel = (id: string) => {
    setDashboardQueries((current) => current.includes(id) ? current.filter((item) => item !== id) : [...current, id]);
  };

  if (workspaceState === "loading" && !workspace) return <AnalyticsLoading label="Loading saved analytics…" />;
  return (
    <>
      <AnalyticsHeading
        eyebrow="Analytics workspace"
        title="Turn useful answers into shared monitoring."
        detail="Save a successful analysis, choose dashboard panels and layout, and configure alerts with explicit thresholds and schedules."
        freshness="Persisted in Rust"
      />
      {feedback && <div className={`notice ${feedback.kind}`} role={feedback.kind === "error" ? "alert" : "status"}><span>{feedback.text}</span></div>}
      <div className="workspace-grid">
        <section className="analytics-surface">
          <div className="surface-heading">
            <div><span className="eyebrow">Reusable analysis</span><h2>Saved queries</h2></div>
            <span className="result-count">{workspace?.saved_queries.length ?? 0}</span>
          </div>
          <form className="compact-create" onSubmit={saveQuery}>
            <label><span className="sr-only">Saved query name</span><input value={queryName} onChange={(event) => setQueryName(event.target.value)} placeholder="Name the current successful analysis" required /></label>
            <button disabled={!canWrite || busy || !currentExecution?.result || currentExecution.state !== "ready"}>Save current</button>
          </form>
          <div className="saved-list">
            {workspace?.saved_queries.length ? workspace.saved_queries.map((query) => (
              <article key={query.id}>
                <div>
                  <span className={`viz-badge ${query.visualization}`}>{query.visualization}</span>
                  <strong>{query.name}</strong>
                  <small>{describeQueryRange(query.plan)} · updated {relativeTime(query.updated_at)}</small>
                </div>
                <div className="row-actions">
                  <button type="button" onClick={() => openQuery(query.plan, query.visualization)}>Open</button>
                  <button type="button" onClick={() => setEditingQuery(query)}>Edit</button>
                  <button type="button" className="danger-text" disabled={!canWrite || busy} onClick={() => archive("saved-query", query.id, query.name)}>Archive</button>
                </div>
              </article>
            )) : <AnalyticsEmpty title="No saved queries" detail="Run an analysis successfully, then save it for the team." />}
          </div>
        </section>

        <section className="analytics-surface dashboard-surface">
          <div className="surface-heading"><div><span className="eyebrow">Team monitoring</span><h2>Dashboards</h2></div></div>
          <form className="dashboard-create" onSubmit={createDashboard}>
            <label><span>Dashboard name</span><input value={dashboardName} onChange={(event) => setDashboardName(event.target.value)} placeholder="Product pulse" required /></label>
            <fieldset>
              <legend>Choose panels and width</legend>
              <div className="panel-picker">
                {workspace?.saved_queries.map((query) => (
                  <div key={query.id}>
                    <label><input type="checkbox" checked={dashboardQueries.includes(query.id)} onChange={() => togglePanel(query.id)} /> {query.name}</label>
                    <label><input type="checkbox" checked={wideQueries.includes(query.id)} disabled={!dashboardQueries.includes(query.id)} onChange={() => setWideQueries((current) => current.includes(query.id) ? current.filter((item) => item !== query.id) : [...current, query.id])} /> Wide</label>
                  </div>
                ))}
              </div>
            </fieldset>
            <button disabled={!canWrite || busy || !dashboardName || !dashboardQueries.length}>Create dashboard</button>
          </form>
          <div className="dashboard-list">
            {workspace?.dashboards.map((dashboard) => (
              <div className="dashboard-manager" key={dashboard.id}>
                <DashboardPreview
                  api={api}
                  project={project}
                  environment={environment}
                  dashboard={dashboard}
                  queries={workspace.saved_queries}
                  onEvidence={onEvidence}
                  onTrace={onTrace}
                  onReplay={onReplay}
                />
                <div className="manager-actions">
                  <button type="button" onClick={() => setEditingDashboard(dashboard)}>Edit layout</button>
                  <button type="button" className="danger-text" disabled={!canWrite || busy} onClick={() => archive("dashboard", dashboard.id, dashboard.name)}>Archive dashboard</button>
                </div>
              </div>
            ))}
          </div>
          {workspace?.dashboards.length === 0 && <AnalyticsEmpty title="No dashboards" detail="Choose one or more saved queries to publish a shared monitoring view." />}
        </section>

        <section className="analytics-surface alert-surface">
          <div className="surface-heading"><div><span className="eyebrow">Scheduled evaluation</span><h2>Alerts</h2></div></div>
          <form className="alert-create" onSubmit={createAlert}>
            <label><span>Saved query</span><select value={effectiveSelectedQuery} onChange={(event) => setSelectedQuery(event.target.value)} required><option value="">Choose query…</option>{workspace?.saved_queries.map((query) => <option value={query.id} key={query.id}>{query.name}</option>)}</select></label>
            <label><span>Alert name</span><input value={alertName} onChange={(event) => setAlertName(event.target.value)} placeholder="Conversion dropped" required /></label>
            <label><span>Operator</span><select value={operator} onChange={(event) => setOperator(event.target.value as AnalyticsAlert["operator"])}><option value="gt">Greater than</option><option value="gte">Greater than or equal</option><option value="lt">Less than</option><option value="lte">Less than or equal</option><option value="eq">Equal to</option></select></label>
            <label><span>Threshold</span><input type="number" step="any" value={threshold} onChange={(event) => setThreshold(event.target.value)} required /></label>
            <label><span>Schedule</span><select value={schedule} onChange={(event) => setSchedule(event.target.value)}><option value="5">Every 5 minutes</option><option value="15">Every 15 minutes</option><option value="60">Hourly</option><option value="1440">Daily</option></select></label>
            <button disabled={!canWrite || busy || !effectiveSelectedQuery}>Create alert</button>
          </form>
          <div className="alert-list">
            {workspace?.alerts.length ? workspace.alerts.map((alert) => (
              <div key={alert.id}>
                <span className={`health-dot ${alert.last_state ?? "active"}`} aria-hidden="true" />
                <div><strong>{alert.name}</strong><span>{alertSummary(alert)}</span></div>
                <div className="row-actions"><time>{alert.last_evaluated_at ? `Checked ${relativeTime(alert.last_evaluated_at)}` : `Due ${relativeTime(alert.next_evaluation_at)}`}</time><button type="button" className="danger-text" disabled={!canWrite || busy} onClick={() => archive("alert", alert.id, alert.name)}>Archive</button></div>
              </div>
            )) : <AnalyticsEmpty title="No alerts" detail="Choose a saved query, threshold, and evaluation schedule." />}
          </div>
        </section>
      </div>

      {editingQuery && (
        <QueryEditor
          query={editingQuery}
          busy={busy}
          onClose={() => setEditingQuery(null)}
          onSave={(next) => void mutate("Saved query updated.", async () => {
            await api.updateSavedQuery(editingQuery.id, next);
            setEditingQuery(null);
          })}
        />
      )}
      {editingDashboard && workspace && (
        <DashboardEditor
          dashboard={editingDashboard}
          queries={workspace.saved_queries}
          busy={busy}
          onClose={() => setEditingDashboard(null)}
          onSave={(next) => void mutate("Dashboard updated.", async () => {
            await api.updateDashboard(editingDashboard.id, next);
            setEditingDashboard(null);
          })}
        />
      )}
    </>
  );
}

function QueryEditor({
  query,
  busy,
  onClose,
  onSave,
}: {
  query: SavedQuery;
  busy: boolean;
  onClose: () => void;
  onSave: (query: Pick<SavedQuery, "name" | "description" | "plan" | "visualization">) => void;
}) {
  const [name, setName] = useState(query.name);
  const [description, setDescription] = useState(query.description);
  const [visualization, setVisualization] = useState(query.visualization);
  return (
    <div className="modal-backdrop">
      <form className="analytics-modal" role="dialog" aria-modal="true" aria-labelledby="query-editor-title" onSubmit={(event) => { event.preventDefault(); onSave({ name, description, plan: query.plan, visualization }); }}>
        <button type="button" className="modal-close" aria-label="Close editor" onClick={onClose}>×</button>
        <span className="eyebrow">Saved query</span>
        <h2 id="query-editor-title">Edit query</h2>
        <label><span>Name</span><input value={name} onChange={(event) => setName(event.target.value)} required /></label>
        <label><span>Description</span><textarea value={description} onChange={(event) => setDescription(event.target.value)} rows={3} /></label>
        <label><span>Visualization</span><select value={visualization} onChange={(event) => setVisualization(event.target.value as Visualization)}>{["table", "line", "bar", "funnel", "retention", "path"].map((value) => <option key={value}>{value}</option>)}</select></label>
        <p className="range-caption">{describeQueryRange(query.plan)}</p>
        <button className="button primary" disabled={busy}>Save changes</button>
      </form>
    </div>
  );
}

function DashboardEditor({
  dashboard,
  queries,
  busy,
  onClose,
  onSave,
}: {
  dashboard: Dashboard;
  queries: SavedQuery[];
  busy: boolean;
  onClose: () => void;
  onSave: (dashboard: Pick<Dashboard, "name" | "description" | "sharing" | "layout">) => void;
}) {
  const [name, setName] = useState(dashboard.name);
  const [description, setDescription] = useState(dashboard.description);
  const [sharing, setSharing] = useState(dashboard.sharing);
  const [layout, setLayout] = useState(dashboard.layout);
  const toggle = (id: string) => {
    setLayout((current) => current.some((item) => item.saved_query_id === id)
      ? current.filter((item) => item.saved_query_id !== id)
      : [...current, { saved_query_id: id, width: 1 }]);
  };
  return (
    <div className="modal-backdrop">
      <form className="analytics-modal" role="dialog" aria-modal="true" aria-labelledby="dashboard-editor-title" onSubmit={(event) => { event.preventDefault(); onSave({ name, description, sharing, layout }); }}>
        <button type="button" className="modal-close" aria-label="Close editor" onClick={onClose}>×</button>
        <span className="eyebrow">Team dashboard</span>
        <h2 id="dashboard-editor-title">Edit dashboard</h2>
        <label><span>Name</span><input value={name} onChange={(event) => setName(event.target.value)} required /></label>
        <label><span>Description</span><textarea value={description} onChange={(event) => setDescription(event.target.value)} rows={3} /></label>
        <label><span>Sharing</span><select value={sharing} onChange={(event) => setSharing(event.target.value as Dashboard["sharing"])}><option value="organization">Organization</option><option value="private">Private</option></select></label>
        <fieldset><legend>Panels and width</legend><div className="panel-picker">{queries.map((query) => {
          const item = layout.find((candidate) => candidate.saved_query_id === query.id);
          return <div key={query.id}><label><input type="checkbox" checked={Boolean(item)} onChange={() => toggle(query.id)} /> {query.name}</label><label><input type="checkbox" checked={item?.width === 2} disabled={!item} onChange={() => setLayout((current) => current.map((candidate) => candidate.saved_query_id === query.id ? { ...candidate, width: candidate.width === 2 ? 1 : 2 } : candidate))} /> Wide</label></div>;
        })}</div></fieldset>
        <button className="button primary" disabled={busy || !layout.length}>Save changes</button>
      </form>
    </div>
  );
}

function DashboardPreview({
  api,
  project,
  environment,
  dashboard,
  queries,
  onEvidence,
  onTrace,
  onReplay,
}: {
  api: ChillApi;
  project: ConsoleProject;
  environment: ConsoleEnvironment;
  dashboard: Dashboard;
  queries: SavedQuery[];
  onEvidence: (evidence: Evidence) => void;
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
}) {
  return (
    <article className="dashboard-preview">
      <header>
        <div><strong>{dashboard.name}</strong><span>{dashboard.description}</span></div>
        <span>{dashboard.layout.length} panels · {dashboard.sharing} · updated {relativeTime(dashboard.updated_at)}</span>
      </header>
      <div className="dashboard-panels">
        {dashboard.layout.slice(0, 8).map((item, index) => {
          const query = queries.find((candidate) => candidate.id === item.saved_query_id);
          return query ? (
            <DashboardQueryPanel
              key={`${item.saved_query_id}-${index}`}
              api={api}
              project={project}
              environment={environment}
              query={query}
              wide={item.width === 2}
              onEvidence={onEvidence}
              onTrace={onTrace}
              onReplay={onReplay}
            />
          ) : null;
        })}
      </div>
    </article>
  );
}

function DashboardQueryPanel({
  api,
  project,
  environment,
  query,
  wide,
  onEvidence,
  onTrace,
  onReplay,
}: {
  api: ChillApi;
  project: ConsoleProject;
  environment: ConsoleEnvironment;
  query: SavedQuery;
  wide: boolean;
  onEvidence: (evidence: Evidence) => void;
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
}) {
  const [result, setResult] = useState<QueryResult | null>(null);
  const [previous, setPrevious] = useState<QueryResult | null>(null);
  const [state, setState] = useState<LoadState>("loading");
  const [error, setError] = useState("");
  const [evaluatedAt, setEvaluatedAt] = useState(() => Date.now());
  useEffect(() => {
    let active = true;
    const now = Date.now();
    const evaluated = resolveQueryPlan(query.plan, now);
    queueMicrotask(() => {
      if (!active) return;
      setState("loading");
      setError("");
    });
    void Promise.all([
      api.query(project.id, environment.id, evaluated),
      api.query(project.id, environment.id, previousPeriodPlan(query.plan, now)),
    ]).then(([current, prior]) => {
      if (!active) return;
      setResult(current);
      setPrevious(prior);
      setEvaluatedAt(now);
      setState("ready");
    }, (cause: unknown) => {
      if (!active) return;
      setState((current) => current === "ready" || current === "stale" ? "stale" : "error");
      setError(message(cause));
    });
    return () => { active = false; };
  }, [api, environment.id, project.id, query]);
  return (
    <section className={`dashboard-panel ${wide ? "wide" : ""} ${state}`}>
      <header>
        <div><span className={`viz-badge ${query.visualization}`}>{query.visualization}</span><strong>{query.name}</strong></div>
        <small>{result ? `${result.stats.row_count} rows · ${comparison(result, previous)}` : state === "error" ? "Unavailable" : "Refreshing…"}</small>
      </header>
      {state === "stale" && <div className="panel-state stale" role="status">Stale · refresh failed</div>}
      {state === "error" ? (
        <AnalyticsEmpty title="Panel unavailable" detail={error} />
      ) : state === "loading" && !result ? (
        <AnalyticsLoading label={`Refreshing ${query.name}…`} compact />
      ) : query.visualization === "funnel" ? (
        <FunnelChart result={result} onTrace={onTrace} onReplay={onReplay} />
      ) : query.visualization === "path" ? (
        <PathChart result={result} onTrace={onTrace} />
      ) : query.visualization === "retention" ? (
        <RetentionMatrix result={result} />
      ) : query.visualization === "line" || query.visualization === "bar" ? (
        <SeriesChart result={result} line={query.visualization === "line"} label={query.name} />
      ) : (
        <DataTable result={result} />
      )}
      <button
        type="button"
        className="panel-evidence"
        disabled={!result}
        onClick={() => onEvidence({
          title: query.name,
          detail: `${result?.stats.row_count ?? 0} rows · ${formatBytes(result?.stats.scan_bytes ?? 0)} scanned. ${describeQueryRange(query.plan, evaluatedAt)}`,
          rows: rowsAsObjects(result).slice(0, 8).map((row, index) => ({
            label: `Row ${index + 1}`,
            value: Object.entries(row).slice(0, 4).map(([key, value]) => `${key}: ${formatCell(value)}`).join(" · "),
          })),
          traceId: firstIdentifier(result, "example_trace_id"),
          sessionId: firstIdentifier(result, "example_session_id"),
        })}
      >
        View evidence
      </button>
    </section>
  );
}

function EvidenceDrawer({
  evidence,
  onClose,
  onTrace,
  onReplay,
}: {
  evidence: Evidence;
  onClose: () => void;
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
}) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);
  return (
    <div className="evidence-backdrop" role="presentation" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}>
      <aside className="evidence-drawer" role="dialog" aria-modal="true" aria-labelledby="evidence-title">
        <header>
          <div><span className="eyebrow">Evidence</span><h2 id="evidence-title">{evidence.title}</h2></div>
          <button type="button" aria-label="Close evidence" onClick={onClose}>×</button>
        </header>
        <p>{evidence.detail}</p>
        {evidence.rows?.length ? <dl>{evidence.rows.map((row, index) => <div key={`${row.label}-${index}`}><dt>{row.label}</dt><dd>{row.value}</dd></div>)}</dl> : <AnalyticsEmpty title="No supporting rows" detail="The current result did not include row-level evidence." />}
        <div className="drawer-actions">
          {evidence.traceId && <button type="button" className="button secondary" onClick={() => onTrace(evidence.traceId ?? "")}>Open trace</button>}
          {evidence.sessionId && <button type="button" className="button secondary" onClick={() => onReplay(evidence.sessionId ?? "")}>Open replay</button>}
        </div>
      </aside>
    </div>
  );
}

function AnalyticsHeading({
  eyebrow,
  title,
  detail,
  freshness,
}: {
  eyebrow: string;
  title: string;
  detail: string;
  freshness: string;
}) {
  return (
    <header className="analytics-heading">
      <div><span className="eyebrow">{eyebrow}</span><h1>{title}</h1><p>{detail}</p></div>
      <div className="freshness-card"><span className="live-pulse" aria-hidden="true" /><strong>{freshness}</strong><span>Times shown in UTC</span></div>
    </header>
  );
}

function BuilderLayout({ controls, children }: { controls: React.ReactNode; children: React.ReactNode }) {
  return <div className="builder-layout"><aside className="analytics-surface builder-sidebar"><span className="eyebrow">Query builder</span>{controls}</aside><section className="analytics-surface analysis-result">{children}</section></div>;
}

function ResultMeta({ execution }: { execution: QueryExecution | null }) {
  return (
    <div className="result-meta">
      <div>
        <span className="eyebrow">Result</span>
        <h2>{execution?.state === "loading" ? "Running analysis…" : execution?.result ? `${execution.result.stats.row_count} rows` : execution?.state === "error" ? "Analysis failed" : "Ready to analyze"}</h2>
        {execution && <small>{describeQueryRange(execution.sourcePlan, execution.evaluatedAt)}</small>}
      </div>
      {execution?.result && (
        <div>
          {execution.state === "stale" && <span className="stale-badge">stale result</span>}
          <span>{formatBytes(execution.result.stats.scan_bytes)} scanned</span>
          <span>{formatDuration(execution.result.stats.total_duration_nano)}</span>
          {execution.result.stats.cache_hit && <span>cache hit</span>}
        </div>
      )}
    </div>
  );
}

function TelemetryRecord({
  record,
  onTrace,
  onReplay,
  expanded = false,
}: {
  record: Record<string, unknown>;
  onTrace: (id: string) => void;
  onReplay: (id: string) => void;
  expanded?: boolean;
}) {
  const envelope = parseEnvelope(record.canonical_json);
  const kind = String(record.kind ?? envelope.kind ?? record.envelope_kind ?? "record");
  const name = String(record.name ?? envelope.name ?? kind);
  const trace = String(record.trace_id ?? envelope.trace?.trace_id ?? "");
  const session = String(record.session_id ?? envelope.context?.session_id ?? "");
  const page = envelope.context?.page?.path;
  return (
    <details className={`telemetry-record kind-${kind}`} open={expanded}>
      <summary>
        <span className="record-glyph" aria-hidden="true">{kind.slice(0, 1).toUpperCase()}</span>
        <div>
          <strong>{name}</strong>
          <span>{kind} · {String(record.operation ?? envelope.operation ?? "instant")}{Array.isArray(page) ? ` · ${page.join(" / ")}` : ""}</span>
        </div>
        <time>{formatNano(record.effective_occurred_at_unix_nano)}</time>
      </summary>
      <div className="record-detail">
        <div className="context-chips">{Object.entries(envelope.annotations ?? {}).slice(0, 8).map(([key, value]) => <span key={key}>{key}: {String(value)}</span>)}</div>
        <pre>{JSON.stringify(envelope, null, 2)}</pre>
        <div className="record-actions">
          {trace && <button type="button" onClick={() => onTrace(trace)}>Open distributed trace</button>}
          {session && <button type="button" onClick={() => onReplay(session)}>Open session replay</button>}
        </div>
      </div>
    </details>
  );
}

function FunnelChart({
  result,
  onTrace,
  onReplay,
}: {
  result: QueryResult | null;
  onTrace?: (id: string) => void;
  onReplay?: (id: string) => void;
}) {
  const rows = rowsAsObjects(result).filter((row) => typeof row.step_label === "string");
  const first = finiteNumber(rows[0]?.subjects) ?? 0;
  return (
    <div className="funnel-chart">
      {rows.length ? rows.map((row, index) => {
        const count = finiteNumber(row.subjects) ?? 0;
        const percent = first > 0 ? count / first * 100 : 0;
        return (
          <div key={index}>
            <div>
              <span>{finiteNumber(row.step_index) ?? index + 1}</span>
              <strong>{String(row.step_label)}{row.breakdown_value && row.breakdown_value !== "all" ? ` · ${String(row.breakdown_value)}` : ""}</strong>
              <small>
                {count.toLocaleString()} subjects · {percent.toFixed(1)}%
                {onTrace && Boolean(row.example_trace_id) && <button type="button" onClick={() => onTrace(String(row.example_trace_id))}>Representative trace</button>}
                {onReplay && Boolean(row.example_session_id) && <button type="button" onClick={() => onReplay(String(row.example_session_id))}>Representative replay</button>}
              </small>
            </div>
            <div className="funnel-bar" role="img" aria-label={`${String(row.step_label)}: ${count} subjects, ${percent.toFixed(1)} percent`}><span style={{ width: `${Math.max(2, Math.min(100, percent))}%` }} /></div>
          </div>
        );
      }) : <AnalyticsEmpty title="Define a funnel" detail="Results show conversion and drop-off at every step." />}
    </div>
  );
}

function PathChart({ result, onTrace }: { result: QueryResult | null; onTrace?: (id: string) => void }) {
  const rows = rowsAsObjects(result).filter((row) => typeof row.from_name === "string" && typeof row.to_name === "string");
  return (
    <div className="path-chart">
      {rows.map((row, index) => (
        <article key={index}>
          <span>{String(row.from_name)}</span>
          <div><strong>{finiteNumber(row.sessions)?.toLocaleString() ?? "0"}</strong><span>sessions to</span></div>
          <span>{String(row.to_name)}</span>
          {onTrace && Boolean(row.example_trace_id) && <button type="button" onClick={() => onTrace(String(row.example_trace_id))}>Open trace</button>}
        </article>
      ))}
      {!rows.length && <AnalyticsEmpty title="Discover a path" detail="Anchor on a semantic event to reveal neighboring behavior." />}
    </div>
  );
}

function SeriesChart({ result, line, label }: { result: QueryResult | null; line: boolean; label: string }) {
  const values = seriesValues(result).slice(0, 32);
  const maximum = Math.max(1, ...values.map((point) => point.value));
  return values.length ? (
    <figure className={`series-chart ${line ? "line" : "bar"}`} aria-label={label}>
      <ol aria-hidden="true">
        {values.map((point, index) => (
          <li key={`${point.label}-${index}`}>
            <button type="button" aria-label={`${point.label}: ${point.value.toLocaleString()}`} style={{ height: `${Math.max(4, point.value / maximum * 100)}%` }}>
              <span>{point.value.toLocaleString()}</span>
            </button>
          </li>
        ))}
      </ol>
      <figcaption>{values[0]?.label}–{values.at(-1)?.label} · peak {maximum.toLocaleString()}</figcaption>
      <table className="sr-only">
        <caption>{label} data</caption>
        <thead><tr><th>Point</th><th>Value</th></tr></thead>
        <tbody>{values.map((point, index) => <tr key={`${point.label}-row-${index}`}><th>{point.label}</th><td>{point.value}</td></tr>)}</tbody>
      </table>
    </figure>
  ) : <AnalyticsEmpty title="No series yet" detail="This panel refreshes from its saved query." />;
}

function RetentionMatrix({ result }: { result: QueryResult | null }) {
  const model = retentionModel(result);
  return model.cohorts.length ? (
    <div className="retention-wrap">
      <table className="retention-table">
        <thead><tr><th>Cohort</th><th>Size</th>{model.periods.map((period) => <th key={period}>P{period}</th>)}</tr></thead>
        <tbody>
          {model.cohorts.map((cohort) => (
            <tr key={cohort.cohort}>
              <th>{cohort.cohort}</th>
              <td>{cohort.size}</td>
              {model.periods.map((period) => {
                const value = cohort.values.get(period);
                return <td key={period}>{value === undefined ? "—" : <span style={{ opacity: .28 + value * .72 }}>{(value * 100).toFixed(1)}%</span>}</td>;
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  ) : <AnalyticsEmpty title="Calculate retention" detail="Choose cohort-forming and return events to build the matrix." />;
}

function DataTable({ result }: { result: QueryResult | null }) {
  if (!result?.rows.length) return <AnalyticsEmpty title="No result yet" detail="Configure and run the analysis." />;
  return (
    <div className="analytics-table-wrap">
      <table className="data-table">
        <thead><tr>{result.columns.map((column) => <th key={column}>{column.replaceAll("_", " ")}</th>)}</tr></thead>
        <tbody>{result.rows.map((row, index) => <tr key={index}>{row.map((value, cell) => <td key={cell}>{formatCell(value)}</td>)}</tr>)}</tbody>
      </table>
    </div>
  );
}

function StructuralFrame({ record }: { record?: Record<string, unknown> }) {
  if (!record) return <div className="replay-empty"><strong>No replay loaded</strong><small>Source-masked structure will appear here without text or pixels.</small></div>;
  const envelope = parseEnvelope(record.canonical_json);
  const payload = envelope.payload ?? envelope;
  return (
    <div className="structural-frame">
      <span className="mask-banner">Masking: {String(envelope.privacy?.redaction_state ?? "source")}</span>
      <pre>{JSON.stringify(payload, null, 2).slice(0, 1_800)}</pre>
    </div>
  );
}

function AnalyticsEmpty({
  title,
  detail,
  action,
}: {
  title: string;
  detail: string;
  action?: React.ReactNode;
}) {
  return <div className="analytics-empty"><span>No data</span><strong>{title}</strong><p>{detail}</p>{action}</div>;
}

function AnalyticsLoading({ label, compact = false }: { label: string; compact?: boolean }) {
  return <div className={`analytics-loading ${compact ? "compact" : ""}`} role="status"><span className="loading-bar" /><strong>{label}</strong></div>;
}

export function planTemplate(kind: AnalyticsKind, values: Record<string, unknown> = {}): Record<string, unknown> {
  const end = Date.now() * 1_000_000;
  const range = {
    start_unix_nano: end - 7 * 86_400_000_000_000,
    end_unix_nano: end,
    relative: { amount: 7, unit: "day" },
  };
  const filter = { behavior_kind: "", operation: "", name: "", annotations: {} };
  const base: Record<string, unknown> = { version: 1, kind, range };
  if (kind === "events") base.events = { filter, session_id: values.sessionId ?? "", trace_id: "", installation_id: "", before: null, limit: 500 };
  if (kind === "trace") base.trace = { trace_id: values.traceId ?? "4bf92f3577b34da6a3ce929d0e0e4736", limit: 500 };
  if (kind === "replay") base.replay = { replay_id: values.replayId ?? "", session_id: values.sessionId ?? "", limit: 1000 };
  if (kind === "funnel") {
    const names = values.names as string[] | undefined ?? ["page.viewed", "purchase.completed"];
    const annotations = values.propertyKey && values.propertyValue ? { [String(values.propertyKey)]: String(values.propertyValue) } : {};
    base.funnel = {
      mode: values.mode ?? "ordered",
      breakdown: values.breakdown ?? "none",
      exclusions: values.exclusion ? [{ label: "excluded", kind: "", operation: "", name: values.exclusion, annotations: {} }] : [],
      steps: names.map((name) => ({ label: name.replaceAll(".", " "), kind: "", operation: "", name, annotations })),
      completion_window_nano: Number(values.windowMinutes ?? 60) * 60_000_000_000,
    };
  }
  if (kind === "path") base.path = { anchor_name: values.anchor ?? "checkout.started", direction: values.direction ?? "after", depth: 3, filter, limit: 250 };
  if (kind === "cohort") {
    const sequence = values.sequence as string[] | undefined ?? [];
    const annotations = values.propertyKey && values.propertyValue ? { [String(values.propertyKey)]: String(values.propertyValue) } : {};
    base.cohort = {
      filter: { ...filter, name: values.name ?? "app.opened", annotations },
      minimum_count: values.minimum ?? 2,
      sequence: sequence.map((name) => ({ label: name.replaceAll(".", " "), kind: "", operation: "", name, annotations })),
      sequence_window_nano: sequence.length ? Number(values.windowMinutes ?? 1440) * 60_000_000_000 : 0,
      limit: 500,
    };
  }
  if (kind === "aggregate") base.aggregate = { metric: values.metric ?? "count", dimension: values.dimension ?? "kind", interval: values.interval ?? "day", filter, limit: 500 };
  if (kind === "retention") base.retention = { start_filter: { ...filter, name: values.start ?? "session.started" }, return_filter: { ...filter, name: values.returnEvent ?? "app.opened" }, interval: "day", periods: 30, limit: 1000 };
  return base;
}

function comparison(current: QueryResult, previous: QueryResult | null): string {
  if (!previous) return "comparison unavailable";
  const now = sumMetric(current);
  const prior = sumMetric(previous);
  if (prior === 0) return now === 0 ? "flat vs previous" : "new vs previous";
  const change = (now - prior) / Math.abs(prior) * 100;
  return `${change >= 0 ? "+" : ""}${change.toFixed(1)}% vs previous`;
}

function comparisonDetail(current: QueryResult | null, previous: QueryResult | null): string {
  if (!current || !previous) return "The comparison will be available after both adjacent ranges finish loading.";
  return `The current rolling range contains ${sumMetric(current).toLocaleString()} records; the immediately preceding range contains ${sumMetric(previous).toLocaleString()}.`;
}

function sumMetric(result: QueryResult | null): number {
  if (!result) return 0;
  return result.rows.reduce((total, row) => {
    const value = [...row].reverse().find((cell) => typeof cell === "number" && Number.isFinite(cell));
    return total + (typeof value === "number" ? value : 0);
  }, 0);
}

type ParsedEnvelope = {
  kind?: unknown;
  name?: unknown;
  operation?: unknown;
  annotations?: Record<string, unknown>;
  trace?: { trace_id?: unknown };
  context?: { session_id?: unknown; page?: { path?: unknown } };
  payload?: Record<string, unknown>;
  privacy?: { redaction_state?: unknown };
};

function parseEnvelope(raw: unknown): ParsedEnvelope {
  if (typeof raw !== "string") return {};
  try {
    const value: unknown = JSON.parse(raw);
    return value && typeof value === "object" ? value as ParsedEnvelope : {};
  } catch {
    return {};
  }
}

function initialIdentifier(plan: Record<string, unknown>): string {
  const trace = objectValue(plan.trace);
  if (trace) return stringValue(trace.trace_id);
  const replay = objectValue(plan.replay);
  if (replay) return stringValue(replay.replay_id) || stringValue(replay.session_id);
  const events = objectValue(plan.events);
  return stringValue(events?.session_id);
}

function initialFunnelSteps(plan: Record<string, unknown>): string {
  const funnel = objectValue(plan.funnel);
  const steps = Array.isArray(funnel?.steps) ? funnel.steps : [];
  const names = steps.flatMap((step) => {
    const name = stringValue(objectValue(step)?.name);
    return name ? [name] : [];
  });
  return names.length ? names.join(", ") : "page.viewed, checkout.started, purchase.completed";
}

function initialPathAnchor(plan: Record<string, unknown>): string {
  return stringValue(objectValue(plan.path)?.anchor_name) || "checkout.started";
}

function initialRetentionStart(plan: Record<string, unknown>): string {
  return stringValue(objectValue(objectValue(plan.retention)?.start_filter)?.name) || "session.started";
}

function initialRetentionReturn(plan: Record<string, unknown>): string {
  return stringValue(objectValue(objectValue(plan.retention)?.return_filter)?.name) || "app.opened";
}

function tabForKind(kind: AnalyticsKind | null): StudioTab {
  if (kind === "trace" || kind === "events") return "journeys";
  if (kind === "funnel" || kind === "path") return "funnels";
  if (kind === "cohort" || kind === "retention") return "cohorts";
  if (kind === "replay") return "replay";
  return "workspace";
}

function planSignature(plan: Record<string, unknown>): string {
  return JSON.stringify(plan);
}

function firstIdentifier(result: QueryResult | null, column: string): string | undefined {
  const index = result?.columns.indexOf(column) ?? -1;
  if (!result || index < 0) return undefined;
  const value = result.rows.find((row) => typeof row[index] === "string" && row[index])?.[index];
  return typeof value === "string" ? value : undefined;
}

function alertSummary(alert: AnalyticsAlert): string {
  const value = alert.last_value === undefined ? "not evaluated" : `last value ${alert.last_value.toLocaleString()}`;
  return `${alert.operator} ${alert.threshold} · ${value} · every ${alert.schedule_minutes} min`;
}

function latestTimestamp(values: Array<string | undefined>): string | undefined {
  return values.filter((value): value is string => Boolean(value)).sort((a, b) => Date.parse(b) - Date.parse(a))[0];
}

function objectValue(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function replayType(record: Record<string, unknown>): string {
  const value = parseEnvelope(record.canonical_json);
  return String(value.payload?.type ?? value.payload?.kind ?? "structure");
}

function formatCell(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value) ?? "";
}

function formatBytes(value: number): string {
  return value < 1024 ? `${value} B` : value < 1_048_576 ? `${(value / 1024).toFixed(1)} KiB` : `${(value / 1_048_576).toFixed(1)} MiB`;
}

function formatDuration(value: number): string {
  return value < 1_000_000 ? `${Math.round(value / 1_000)} μs` : `${(value / 1_000_000).toFixed(1)} ms`;
}

function formatNano(value: unknown): string {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return "—";
  return new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit", timeZone: "UTC" }).format(new Date(numeric / 1_000_000));
}

function formatCompact(value: number): string {
  return new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(value);
}

function relativeTime(value?: string): string {
  if (!value) return "never";
  const delta = Date.now() - new Date(value).valueOf();
  if (!Number.isFinite(delta)) return "unknown";
  if (Math.abs(delta) < 60_000) return "now";
  if (Math.abs(delta) < 3_600_000) return `${Math.round(Math.abs(delta) / 60_000)}m ${delta >= 0 ? "ago" : "from now"}`;
  if (Math.abs(delta) < 86_400_000) return `${Math.round(Math.abs(delta) / 3_600_000)}h ${delta >= 0 ? "ago" : "from now"}`;
  return `${Math.round(Math.abs(delta) / 86_400_000)}d ${delta >= 0 ? "ago" : "from now"}`;
}

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : "The analytics request failed.";
}
