import {
  type FormEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import type { ChillBrowser } from "@chill-observability/browser";
import { ChillApi } from "./api";
import { AnalyticsStudio, type AnalyticsConnectionState } from "./Analytics";
import {
  SESSION_KEYS,
  bootstrapPrivateSession,
  credentialPreview,
  normalizeApiBase,
  parseJsonObject,
  samplingPercentage,
} from "./api-contract";
import {
  closeConsoleTelemetry,
  controlActionName,
  flushConsoleTelemetry,
  getConsoleTelemetry,
} from "./telemetry";
import type {
  Capability,
  ConsoleEnvironment,
  ConsoleOverview,
  ConsoleProject,
  ConsoleSdkKey,
} from "./types";

type View = "overview" | "explore" | "sources" | "keys" | "schemas" | "controls";
type Notice = { kind: "success" | "error"; message: string };
type Secret = { title: string; credential: string };

const navigation: { id: View; label: string; icon: IconName }[] = [
  { id: "overview", label: "Project overview", icon: "home" },
  { id: "explore", label: "Explore telemetry", icon: "chart" },
  { id: "sources", label: "Data sources", icon: "source" },
  { id: "keys", label: "API keys", icon: "key" },
  { id: "schemas", label: "Schema catalog", icon: "schema" },
  { id: "controls", label: "Collection controls", icon: "shield" },
];

export function App() {
  const [session, setSession] = useState(() => sessionStorage.getItem(SESSION_KEYS.token) ?? "");
  const [apiBase, setApiBase] = useState(() => sessionStorage.getItem(SESSION_KEYS.apiBase) ?? "");
  const [overview, setOverview] = useState<ConsoleOverview | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState("");
  const [view, setView] = useState<View>("overview");
  const [projectPreference, setProjectPreference] = useState(
    () => sessionStorage.getItem(SESSION_KEYS.project) ?? "",
  );
  const [environmentPreference, setEnvironmentPreference] = useState(
    () => sessionStorage.getItem(SESSION_KEYS.environment) ?? "",
  );
  const [pendingAction, setPendingAction] = useState("");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [secret, setSecret] = useState<Secret | null>(null);
  const [scopeDialog, setScopeDialog] = useState<"project" | "environment" | null>(null);
  const [automaticLoginPending, setAutomaticLoginPending] = useState(
    () => !sessionStorage.getItem(SESSION_KEYS.token),
  );
  const [automaticLoginError, setAutomaticLoginError] = useState("");
  const [telemetry, setTelemetry] = useState<ChillBrowser | null>(null);
  const [analyticsConnection, setAnalyticsConnection] = useState<AnalyticsConnectionState>("loading");

  useEffect(() => {
    if (sessionStorage.getItem(SESSION_KEYS.token)) return;
    const controller = new AbortController();
    void bootstrapPrivateSession(controller.signal)
      .then((issued) => {
        sessionStorage.setItem(SESSION_KEYS.apiBase, issued.apiBase);
        sessionStorage.setItem(SESSION_KEYS.token, issued.credential);
        setApiBase(issued.apiBase);
        setSession(issued.credential);
      })
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError") return;
        setAutomaticLoginError(messageFrom(error));
      })
      .finally(() => {
        if (!controller.signal.aborted) setAutomaticLoginPending(false);
      });
    return () => controller.abort();
  }, []);

  const api = useMemo(
    () => (session ? new ChillApi(apiBase, session) : null),
    [apiBase, session],
  );

  const loadOverview = useCallback(
    async (signal?: AbortSignal) => {
      if (!api) return;
      setLoading(true);
      setLoadError("");
      try {
        const nextOverview = await api.overview(signal);
        setOverview(nextOverview);
        const nextProject = selectProject(nextOverview, projectPreference);
        const nextEnvironment = selectEnvironment(nextProject, environmentPreference);
        const policyVersion = nextEnvironment?.privacy?.document.policy_version;
        if (typeof policyVersion === "string" && policyVersion) {
          setTelemetry(getConsoleTelemetry(policyVersion));
        }
      } catch (error) {
        if (error instanceof DOMException && error.name === "AbortError") return;
        setLoadError(messageFrom(error));
      } finally {
        if (!signal?.aborted) setLoading(false);
      }
    },
    [api, environmentPreference, projectPreference],
  );

  useEffect(() => {
    if (!api) return;
    const controller = new AbortController();
    queueMicrotask(() => void loadOverview(controller.signal));
    return () => controller.abort();
  }, [api, loadOverview]);

  const selectedProject = selectProject(overview, projectPreference);
  const selectedEnvironment = selectEnvironment(selectedProject, environmentPreference);

  const chooseProject = (projectId: string) => {
    setProjectPreference(projectId);
    sessionStorage.setItem(SESSION_KEYS.project, projectId);
    const project = overview?.projects.find((candidate) => candidate.id === projectId);
    const firstEnvironment = project?.environments[0]?.id ?? "";
    setEnvironmentPreference(firstEnvironment);
    sessionStorage.setItem(SESSION_KEYS.environment, firstEnvironment);
    setAnalyticsConnection("loading");
  };

  const chooseEnvironment = (environmentId: string) => {
    setEnvironmentPreference(environmentId);
    sessionStorage.setItem(SESSION_KEYS.environment, environmentId);
    setAnalyticsConnection("loading");
  };

  const navigate = (destination: View) => {
    telemetry?.event(`console.navigate.${destination}`, canonicalEvent("lifecycle"));
    setView(destination);
  };

  const signIn = (nextApiBase: string, nextSession: string) => {
    const normalizedBase = normalizeApiBase(nextApiBase);
    const normalizedSession = nextSession.trim();
    if (!normalizedSession.startsWith("ch_us_") || normalizedSession.length < 20) {
      throw new Error("Enter a valid ch_us_ user-session credential.");
    }
    sessionStorage.setItem(SESSION_KEYS.apiBase, normalizedBase);
    sessionStorage.setItem(SESSION_KEYS.token, normalizedSession);
    setApiBase(normalizedBase);
    setSession(normalizedSession);
    setOverview(null);
  };

  const signOut = () => {
    telemetry?.event("console.sign_out", canonicalEvent("lifecycle", "terminal"));
    void closeConsoleTelemetry();
    for (const key of Object.values(SESSION_KEYS)) sessionStorage.removeItem(key);
    setSession("");
    setApiBase("");
    setOverview(null);
    setLoadError("");
    setProjectPreference("");
    setEnvironmentPreference("");
    setNotice(null);
    setSecret(null);
    setTelemetry(null);
  };

  const runAction = async (
    label: string,
    operation: () => Promise<void>,
    successMessage: string,
  ) => {
    setPendingAction(label);
    setNotice(null);
    try {
      const execute = async () => {
        await operation();
        await loadOverview();
      };
      await execute();
      telemetry?.event(`${controlActionName(label)}.succeeded`, canonicalEvent("domain", "succeeded"));
      setNotice({ kind: "success", message: successMessage });
    } catch (error) {
      telemetry?.event(`${controlActionName(label)}.failed`, canonicalEvent("error", "terminal"));
      setNotice({ kind: "error", message: messageFrom(error) });
      throw error;
    } finally {
      setPendingAction("");
      void flushConsoleTelemetry();
    }
  };

  useEffect(() => {
    if (!telemetry) return;
    telemetry.startPage(view);
    void flushConsoleTelemetry();
  }, [telemetry, view]);

  useEffect(() => {
    if (!telemetry) return;
    const flush = () => { void flushConsoleTelemetry(); };
    const flushWhenHidden = () => { if (document.visibilityState === "hidden") flush(); };
    const interval = window.setInterval(flush, 15_000);
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", flushWhenHidden);
    return () => {
      window.clearInterval(interval);
      window.removeEventListener("pagehide", flush);
      document.removeEventListener("visibilitychange", flushWhenHidden);
    };
  }, [telemetry]);

  const refreshOverview = () => {
    telemetry?.event("console.refresh", canonicalEvent("lifecycle"));
    void loadOverview().finally(() => flushConsoleTelemetry());
  };

  if (!session && automaticLoginPending) return <AutomaticLoginGate />;
  if (!session) return <SignIn automaticError={automaticLoginError} onSignIn={signIn} />;

  if (!overview) {
    return (
      <LoadingGate
        error={loadError}
        loading={loading}
        onRetry={() => void loadOverview()}
        onSignOut={signOut}
      />
    );
  }

  const canWrite = overview.actor.capabilities.includes("control:write");
  const canManageKeys = overview.actor.capabilities.includes("credentials:manage");
  const canReadData = overview.actor.capabilities.includes("data:read");
  const connectionIssue = Boolean(loadError)
    || (view === "explore" && (analyticsConnection === "error" || analyticsConnection === "stale"));
  const connectionLabel = loadError || (view === "explore" && analyticsConnection === "error")
    ? "API issue"
    : view === "explore" && analyticsConnection === "stale"
      ? "Stale data"
      : view === "explore" && analyticsConnection === "loading"
        ? "Connecting"
        : "Connected";

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">Skip to content</a>
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true"><span /></span>
          <span>chill</span>
        </div>
        <div className="organization-card">
          <span className="eyebrow">Organization</span>
          <strong>{overview.organization.name}</strong>
          <span>{overview.organization.slug}</span>
        </div>
        <nav aria-label="Console navigation">
          {navigation.map((item) => (
            <button
              className={view === item.id ? "nav-item active" : "nav-item"}
              key={item.id}
              onClick={() => navigate(item.id)}
              type="button"
            >
              <Icon name={item.icon} />
              <span>{item.label}</span>
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          <div className="avatar" aria-hidden="true">{initials(overview.actor.display_name)}</div>
          <div className="actor-summary">
            <strong>{overview.actor.display_name}</strong>
            <span>{overview.actor.role}</span>
          </div>
          <button className="icon-button" type="button" onClick={signOut} aria-label="Sign out">
            <Icon name="logout" />
          </button>
        </div>
      </aside>

      <div className="workspace">
        <header className="topbar">
          <div className="mobile-brand"><span className="brand-mark" aria-hidden="true"><span /></span>chill</div>
          <ScopeSelector
            overview={overview}
            project={selectedProject}
            environment={selectedEnvironment}
            onProject={chooseProject}
            onEnvironment={chooseEnvironment}
            canWrite={canWrite}
            onAddProject={() => setScopeDialog("project")}
            onAddEnvironment={() => setScopeDialog("environment")}
          />
          <div className="topbar-actions">
            <span className={connectionIssue ? "status-dot warning" : "status-dot"} aria-hidden="true" />
            <span className="api-status">{connectionLabel}</span>
            <button className="icon-button" type="button" onClick={refreshOverview} aria-label="Refresh console">
              <Icon name="refresh" />
            </button>
          </div>
        </header>

        <main id="main-content" tabIndex={-1}>
          {notice && <NoticeBanner notice={notice} onDismiss={() => setNotice(null)} />}
          {!selectedProject ? (
            <EmptyState title="No projects yet" detail="Bootstrap a project with chillctl, then refresh this console." />
          ) : !selectedEnvironment ? (
            <EmptyState title="No environments yet" detail="This project does not have an environment to configure." />
          ) : (
            <>
              {view === "overview" && (
                <OverviewPage
                  apiBase={apiBase}
                  project={selectedProject}
                  environment={selectedEnvironment}
                  onNavigate={navigate}
                />
              )}
              {view === "sources" && api && (
                <SourcesPage
                  key={selectedEnvironment.id}
                  api={api}
                  apiBase={apiBase}
                  project={selectedProject}
                  environment={selectedEnvironment}
                  canWrite={canWrite}
                  pending={pendingAction}
                  runAction={runAction}
                />
              )}
              {view === "explore" && api && (
                <ExplorePage
                  key={selectedEnvironment.id}
                  api={api}
                  project={selectedProject}
                  environment={selectedEnvironment}
                  canRead={canReadData}
                  canWrite={canWrite}
                  onConnectionState={setAnalyticsConnection}
                />
              )}
              {view === "keys" && api && (
                <KeysPage
                  key={selectedEnvironment.id}
                  api={api}
                  project={selectedProject}
                  environment={selectedEnvironment}
                  canManage={canManageKeys}
                  pending={pendingAction}
                  runAction={runAction}
                  reveal={setSecret}
                />
              )}
              {view === "schemas" && api && (
                <SchemasPage
                  key={selectedProject.id}
                  api={api}
                  project={selectedProject}
                  canWrite={canWrite}
                  pending={pendingAction}
                  runAction={runAction}
                />
              )}
              {view === "controls" && api && (
                <ControlsPage
                  key={selectedEnvironment.id}
                  api={api}
                  project={selectedProject}
                  environment={selectedEnvironment}
                  canWrite={canWrite}
                  pending={pendingAction}
                  runAction={runAction}
                />
              )}
            </>
          )}
        </main>
      </div>
      {secret && <SecretReveal secret={secret} onClose={() => setSecret(null)} />}
      {scopeDialog && api && (
        <ScopeCreator
          kind={scopeDialog}
          api={api}
          project={selectedProject}
          pending={pendingAction}
          runAction={runAction}
          onCreated={(projectId, environmentId) => {
            if (projectId) {
              setProjectPreference(projectId);
              sessionStorage.setItem(SESSION_KEYS.project, projectId);
            }
            if (environmentId) {
              setEnvironmentPreference(environmentId);
              sessionStorage.setItem(SESSION_KEYS.environment, environmentId);
            }
            setScopeDialog(null);
          }}
          onClose={() => setScopeDialog(null)}
        />
      )}
      <div className="sr-only" role="status" aria-live="polite">
        {pendingAction ? `${pendingAction} in progress` : notice?.message}
      </div>
    </div>
  );
}

function AutomaticLoginGate() {
  return (
    <main className="loading-gate">
      <div className="brand brand-large"><span className="brand-mark" aria-hidden="true"><span /></span><span>chill</span></div>
      <div className="loader" aria-hidden="true" />
      <div><h1>Signing you in</h1><p>Verifying your private workspace identity…</p></div>
      <div className="sr-only" role="status" aria-live="polite">Signing in to Chill</div>
    </main>
  );
}

function SignIn({
  automaticError,
  onSignIn,
}: {
  automaticError: string;
  onSignIn: (apiBase: string, token: string) => void;
}) {
  const [apiBase, setApiBase] = useState(() => sessionStorage.getItem(SESSION_KEYS.apiBase) ?? "");
  const [token, setToken] = useState("");
  const [error, setError] = useState("");

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setError("");
    try {
      onSignIn(apiBase, token);
    } catch (cause) {
      setError(messageFrom(cause));
    }
  };

  return (
    <main className="sign-in-page">
      <section className="sign-in-story" aria-label="About Chill">
        <div className="brand brand-large"><span className="brand-mark" aria-hidden="true"><span /></span><span>chill</span></div>
        <div className="sign-in-copy">
          <span className="eyebrow light">Project console</span>
          <h1>See the product.<br />Shape the signal.</h1>
          <p>Configure telemetry with deliberate privacy, predictable sampling, and credentials that stay in your control.</p>
        </div>
        <div className="signal-art" aria-hidden="true">
          <span className="signal-line one" /><span className="signal-line two" /><span className="signal-line three" />
          <span className="signal-node a" /><span className="signal-node b" /><span className="signal-node c" />
        </div>
        <p className="sign-in-footnote">Internal access · User sessions are stored only in this browser tab.</p>
      </section>
      <section className="sign-in-panel">
        <form className="sign-in-form" onSubmit={submit}>
          <span className="eyebrow">Welcome back</span>
          <h2>Connect to Chill</h2>
          <p>Private access connects automatically. Use an operator session only for recovery.</p>
          {(error || automaticError) && <div className="form-error" role="alert"><Icon name="alert" />{error || automaticError}</div>}
          <label>
            <span>Rust API address</span>
            <input
              type="text"
              inputMode="url"
              autoComplete="url"
              placeholder="Same origin (recommended)"
              value={apiBase}
              onChange={(event) => setApiBase(event.target.value)}
            />
            <small>Leave blank when the console and chilld share an origin.</small>
          </label>
          <label>
            <span>User-session credential</span>
            <input
              type="password"
              autoComplete="off"
              spellCheck={false}
              placeholder="ch_us_…"
              value={token}
              onChange={(event) => setToken(event.target.value)}
              required
            />
          </label>
          <button className="button primary large" type="submit">Open console <Icon name="arrow" /></button>
          <div className="privacy-note"><Icon name="lock" /><span>The credential is never written to localStorage, cookies, logs, or a server-side JavaScript runtime.</span></div>
        </form>
      </section>
    </main>
  );
}

function LoadingGate({
  error,
  loading,
  onRetry,
  onSignOut,
}: {
  error: string;
  loading: boolean;
  onRetry: () => void;
  onSignOut: () => void;
}) {
  return (
    <main className="loading-gate">
      <div className="brand brand-large"><span className="brand-mark" aria-hidden="true"><span /></span><span>chill</span></div>
      {error ? (
        <section className="connection-error">
          <span className="error-mark"><Icon name="alert" /></span>
          <h1>Couldn’t open the console</h1>
          <p>{error}</p>
          <div className="button-row"><button className="button primary" onClick={onRetry} type="button">Try again</button><button className="button secondary" onClick={onSignOut} type="button">Change connection</button></div>
        </section>
      ) : (
        <section className="loading-card" aria-live="polite">
          <span className="spinner" aria-hidden="true" />
          <h1>{loading ? "Loading your organization" : "Connecting"}</h1>
          <p>Reading projects and active collection policy from Chill.</p>
        </section>
      )}
    </main>
  );
}

function ScopeSelector({
  overview,
  project,
  environment,
  onProject,
  onEnvironment,
  canWrite,
  onAddProject,
  onAddEnvironment,
}: {
  overview: ConsoleOverview;
  project: ConsoleProject | null;
  environment: ConsoleEnvironment | null;
  onProject: (id: string) => void;
  onEnvironment: (id: string) => void;
  canWrite: boolean;
  onAddProject: () => void;
  onAddEnvironment: () => void;
}) {
  return (
    <div className="scope-selector">
      <label><span className="sr-only">Project</span><Icon name="project" /><select value={project?.id ?? ""} onChange={(event) => onProject(event.target.value)}>
        {overview.projects.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
      </select></label>
      <span className="scope-separator">/</span>
      <label><span className="sr-only">Environment</span><select value={environment?.id ?? ""} onChange={(event) => onEnvironment(event.target.value)} disabled={!project?.environments.length}>
        {project?.environments.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
      </select></label>
      {environment && <span className={`environment-pill ${environment.kind}`}>{environment.kind}</span>}
      <div className="scope-actions">
        <button type="button" onClick={onAddProject} disabled={!canWrite} title="Create project" aria-label="Create project">+</button>
        <button type="button" onClick={onAddEnvironment} disabled={!canWrite || !project} title="Create environment">+ env</button>
      </div>
    </div>
  );
}

function ScopeCreator({ kind, api, project, pending, runAction, onCreated, onClose }: { kind: "project" | "environment"; api: ChillApi; project: ConsoleProject | null; pending: string; runAction: ActionRunner; onCreated: (projectId?: string, environmentId?: string) => void; onClose: () => void }) {
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [environmentKind, setEnvironmentKind] = useState("development");
  const [retention, setRetention] = useState("30");
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    try {
      if (kind === "project") {
        let createdId = "";
        await runAction("Creating project", async () => { createdId = (await api.createProject({ name, slug })).id; }, `${name} was created.`);
        if (createdId) onCreated(createdId);
      } else if (project) {
        let createdId = "";
        await runAction("Creating environment", async () => { createdId = (await api.createEnvironment({ project_id: project.id, name, slug, kind: environmentKind, retention_days: Number(retention) })).id; }, `${name} was created.`);
        if (createdId) onCreated(project.id, createdId);
      }
    } catch { /* shared notice */ }
  };
  return <div className="modal-backdrop" role="presentation"><section className="scope-modal" role="dialog" aria-modal="true" aria-labelledby="scope-title"><button className="modal-close" type="button" onClick={onClose} aria-label="Close">×</button><span className="eyebrow">New {kind}</span><h2 id="scope-title">Create {kind === "project" ? "a project" : `an environment in ${project?.name ?? "project"}`}</h2><p>{kind === "project" ? "Projects group schemas and deployment environments." : "Environments isolate credentials, policy, and retained data."}</p><form className="stack-form" onSubmit={(event) => void submit(event)}><label><span>Name</span><input value={name} onChange={(event) => { setName(event.target.value); if (!slug) setSlug(slugify(event.target.value)); }} maxLength={160} required autoFocus /></label><label><span>Slug</span><input value={slug} onChange={(event) => setSlug(event.target.value.toLowerCase())} pattern="[a-z][a-z0-9-]{1,62}[a-z0-9]" minLength={3} maxLength={64} required /><small>Stable URL-safe identifier; cannot be changed later.</small></label>{kind === "environment" && <><label><span>Class</span><select value={environmentKind} onChange={(event) => setEnvironmentKind(event.target.value)}>{["production", "staging", "development", "test"].map((value) => <option key={value}>{value}</option>)}</select></label><label><span>Retention days</span><input type="number" min={1} max={3650} value={retention} onChange={(event) => setRetention(event.target.value)} required /></label></>}<div className="button-row"><button className="button secondary" type="button" onClick={onClose}>Cancel</button><button className="button primary" type="submit" disabled={!!pending}>{pending ? "Creating…" : `Create ${kind}`}</button></div></form></section></div>;
}

function PageHeading({ eyebrow, title, detail, action }: { eyebrow: string; title: string; detail: string; action?: ReactNode }) {
  return <header className="page-heading"><div><span className="eyebrow">{eyebrow}</span><h1>{title}</h1><p>{detail}</p></div>{action}</header>;
}

function OverviewPage({ apiBase, project, environment, onNavigate }: { apiBase: string; project: ConsoleProject; environment: ConsoleEnvironment; onNavigate: (view: View) => void }) {
  const sampling = environment.sampling;
  const healthItems = [
    { label: "Data sources", value: environment.data_sources.length, detail: environment.data_sources.length ? "Ready to ingest" : "Setup needed", view: "sources" as View },
    { label: "Active keys", value: environment.sdk_keys.filter((key) => key.status === "active").length, detail: "Environment scoped", view: "keys" as View },
    { label: "Schema versions", value: project.schemas.length, detail: project.schemas[0]?.version ?? "None registered", view: "schemas" as View },
    { label: "Retention", value: `${environment.retention_days}d`, detail: "Behavior data", view: "controls" as View },
  ];
  return (
    <div className="page">
      <PageHeading eyebrow="Project overview" title={project.name} detail={`Configuration for ${environment.name}. Changes are applied by the Rust control plane and protected by tenant isolation.`} />
      <section className="metric-grid" aria-label="Environment summary">
        {healthItems.map((item) => <button className="metric-card" key={item.label} type="button" onClick={() => onNavigate(item.view)}><span>{item.label}</span><strong>{item.value}</strong><small>{item.detail}</small><Icon name="arrow" /></button>)}
      </section>
      <div className="overview-grid">
        <section className="panel setup-panel">
          <div className="panel-heading"><div><span className="eyebrow">Quick start</span><h2>Send your first signal</h2></div><span className="step-count">3 steps</span></div>
          <ol className="setup-steps">
            <li className={environment.data_sources.length ? "done" : ""}><span className="step-marker">{environment.data_sources.length ? <Icon name="check" /> : "1"}</span><div><strong>Register a data source</strong><p>Name the client or service that will send telemetry.</p></div><button type="button" onClick={() => onNavigate("sources")}>{environment.data_sources.length ? "Review" : "Configure"}</button></li>
            <li className={environment.sdk_keys.some((key) => key.status === "active") ? "done" : ""}><span className="step-marker">{environment.sdk_keys.some((key) => key.status === "active") ? <Icon name="check" /> : "2"}</span><div><strong>Issue an SDK key</strong><p>Choose the minimum OTLP and replay scopes required.</p></div><button type="button" onClick={() => onNavigate("keys")}>Manage</button></li>
            <li className={sampling && environment.privacy ? "done" : ""}><span className="step-marker">{sampling && environment.privacy ? <Icon name="check" /> : "3"}</span><div><strong>Confirm collection controls</strong><p>Review privacy, sampling, and retention before launch.</p></div><button type="button" onClick={() => onNavigate("controls")}>Review</button></li>
          </ol>
        </section>
        <section className="panel policy-summary">
          <div className="panel-heading"><div><span className="eyebrow">Effective policy</span><h2>Collection posture</h2></div><span className={environment.privacy ? "policy-state safe" : "policy-state warning"}>{environment.privacy ? "Protected" : "Review"}</span></div>
          <dl>
            <div><dt>Behavior sampling</dt><dd>{sampling ? samplingPercentage(sampling.behavior_numerator, sampling.behavior_denominator) : "Not configured"}</dd></div>
            <div><dt>Replay sampling</dt><dd>{sampling ? samplingPercentage(sampling.replay_numerator, sampling.replay_denominator) : "Not configured"}</dd></div>
            <div><dt>Privacy policy</dt><dd>{environment.privacy ? `Version ${environment.privacy.version}` : "Not configured"}</dd></div>
            <div><dt>API origin</dt><dd className="mono">{apiBase || "Same origin"}</dd></div>
          </dl>
          <button className="text-button" type="button" onClick={() => onNavigate("controls")}>Open collection controls <Icon name="arrow" /></button>
        </section>
      </div>
    </div>
  );
}

type ActionRunner = (label: string, operation: () => Promise<void>, success: string) => Promise<void>;

function ExplorePage({ api, project, environment, canRead, canWrite, onConnectionState }: { api: ChillApi; project: ConsoleProject; environment: ConsoleEnvironment; canRead: boolean; canWrite: boolean; onConnectionState: (state: AnalyticsConnectionState) => void }) {
  return <AnalyticsStudio api={api} project={project} environment={environment} canRead={canRead} canWrite={canWrite} onConnectionState={onConnectionState} />;
}

function SourcesPage({ api, apiBase, project, environment, canWrite, pending, runAction }: { api: ChillApi; apiBase: string; project: ConsoleProject; environment: ConsoleEnvironment; canWrite: boolean; pending: string; runAction: ActionRunner }) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState("apple");
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    try {
      await runAction("Creating data source", async () => { await api.createDataSource({ project_id: project.id, environment_id: environment.id, name, kind }); }, `${name} is ready to connect.`);
      setName("");
    } catch { /* surfaced by the shared notice */ }
  };
  return <div className="page"><PageHeading eyebrow="Ingestion" title="Data sources" detail="Register each application or service independently so keys, quotas, and diagnostics retain a clear owner." />
    <div className="split-layout"><section className="panel"><div className="panel-heading"><div><h2>Registered sources</h2><p>{environment.data_sources.length} in {environment.name}</p></div></div>
      {environment.data_sources.length ? <div className="item-list">{environment.data_sources.map((source) => <div className="list-item" key={source.id}><span className={`source-icon ${source.kind}`}><Icon name="source" /></span><div className="item-primary"><strong>{source.name}</strong><span className="mono subtle">{source.id}</span></div><span className="kind-label">{source.kind}</span><span className={`state-label ${source.status}`}>{source.status}</span></div>)}</div> : <EmptyInline icon="source" title="No sources registered" detail="Add the first client or OTLP producer for this environment." />}
    </section><section className="panel form-panel"><div className="panel-heading"><div><h2>Add a data source</h2><p>Names must be unique within this environment.</p></div></div><PermissionNote capability="control:write" allowed={canWrite} />
      <form className="stack-form" onSubmit={(event) => void submit(event)}><label><span>Source name</span><input value={name} onChange={(event) => setName(event.target.value)} placeholder="iOS production app" maxLength={160} required disabled={!canWrite} /></label><label><span>Platform</span><select value={kind} onChange={(event) => setKind(event.target.value)} disabled={!canWrite}>{["apple", "android", "web", "server", "otlp"].map((value) => <option key={value} value={value}>{labelKind(value)}</option>)}</select></label><button className="button primary" type="submit" disabled={!canWrite || !!pending}>{pending === "Creating data source" ? "Creating…" : "Register source"}</button></form>
    </section></div><ConnectionGuide apiBase={apiBase} environment={environment} />
  </div>;
}

function ConnectionGuide({ apiBase, environment }: { apiBase: string; environment: ConsoleEnvironment }) {
  const source = environment.data_sources[0];
  const origin = apiBase || window.location.origin;
  const endpoint = `${origin}/v1/logs`;
  const snippet = source?.kind === "apple"
    ? `let export = ChillOTLPConfiguration(\n  endpoint: URL(string: "${endpoint}")!,\n  headers: ["authorization": "Bearer <CHILL_SDK_KEY>"]\n)`
    : `curl --request POST '${endpoint}' \\\n+  --header 'Authorization: Bearer <CHILL_SDK_KEY>' \\\n+  --header 'Content-Type: application/x-protobuf' \\\n+  --data-binary @otlp-logs.bin`;
  return <section className="panel connection-guide"><div className="panel-heading"><div><span className="eyebrow">SDK setup</span><h2>Connect {source?.name ?? "your first source"}</h2><p>Use an active environment key and send OTLP logs directly to the Rust ingestion endpoint.</p></div><span className="environment-pill">{source?.kind ?? "OTLP"}</span></div><div className="endpoint-row"><span>Endpoint</span><code>{endpoint}</code></div><pre><code>{snippet}</code></pre><p className="guide-note"><Icon name="lock" /> Replace the placeholder at runtime from secure configuration. Never commit an SDK key to source control.</p></section>;
}

function KeysPage({ api, project, environment, canManage, pending, runAction, reveal }: { api: ChillApi; project: ConsoleProject; environment: ConsoleEnvironment; canManage: boolean; pending: string; runAction: ActionRunner; reveal: (secret: Secret) => void }) {
  const [name, setName] = useState("");
  const [sourceId, setSourceId] = useState(environment.data_sources[0]?.id ?? "");
  const [otlp, setOtlp] = useState(true);
  const [replay, setReplay] = useState(false);
  const [expires, setExpires] = useState("");
  const create = async (event: FormEvent) => {
    event.preventDefault();
    const scopes = [otlp ? "ingest:otlp" : "", replay ? "ingest:replay" : ""].filter(Boolean);
    if (!scopes.length) return;
    try {
      let credential = "";
      await runAction("Issuing SDK key", async () => { credential = (await api.createSdkKey({ project_id: project.id, environment_id: environment.id, data_source_id: sourceId, name, scopes, expires_at: expires ? new Date(expires).toISOString() : null })).credential; }, `${name} was issued.`);
      if (credential) reveal({ title: "SDK key issued", credential });
      setName(""); setExpires("");
    } catch { /* surfaced by the shared notice */ }
  };
  const rotate = async (key: ConsoleSdkKey) => {
    if (!window.confirm(`Rotate “${key.name}”? The current credential will stop working immediately.`)) return;
    try {
      let credential = "";
      await runAction("Rotating SDK key", async () => { credential = (await api.rotateSdkKey(key.id)).credential; }, `${key.name} was rotated.`);
      if (credential) reveal({ title: "Replacement key issued", credential });
    } catch { /* surfaced by the shared notice */ }
  };
  const revoke = async (key: ConsoleSdkKey) => {
    if (!window.confirm(`Revoke “${key.name}”? Ingestion using this credential will stop immediately.`)) return;
    try { await runAction("Revoking SDK key", () => api.revokeSdkKey(key.id), `${key.name} was revoked.`); } catch { /* surfaced */ }
  };
  return <div className="page"><PageHeading eyebrow="Credentials" title="API keys" detail="Issue environment-scoped keys, disclose each secret once, and rotate or revoke access immediately." />
    <section className="panel"><div className="panel-heading"><div><h2>SDK keys</h2><p>Secrets are never returned in this list.</p></div><span className="count-badge">{environment.sdk_keys.length}</span></div><PermissionNote capability="credentials:manage" allowed={canManage} />
      {environment.sdk_keys.length ? <div className="key-table-wrap"><table className="data-table"><thead><tr><th>Name</th><th>Prefix</th><th>Scopes</th><th>Last used</th><th>Status</th><th><span className="sr-only">Actions</span></th></tr></thead><tbody>{environment.sdk_keys.map((key) => <tr key={key.id}><td><strong>{key.name}</strong><small>{sourceName(environment, key.data_source_id)}</small></td><td className="mono">{key.prefix}</td><td><div className="tag-row">{key.scopes.map((scope) => <span className="tag" key={scope}>{scope.replace("ingest:", "")}</span>)}</div></td><td>{formatDate(key.last_used_at)}</td><td><span className={`state-label ${key.status}`}>{key.status}</span></td><td><div className="row-actions"><button type="button" onClick={() => void rotate(key)} disabled={!canManage || key.status !== "active" || !!pending}>Rotate</button><button className="danger-text" type="button" onClick={() => void revoke(key)} disabled={!canManage || key.status !== "active" || !!pending}>Revoke</button></div></td></tr>)}</tbody></table></div> : <EmptyInline icon="key" title="No keys issued" detail="Create a scoped credential after registering a data source." />}
    </section>
    <section className="panel form-panel wide-form"><div className="panel-heading"><div><h2>Issue a new key</h2><p>The complete credential will be shown exactly once.</p></div></div><form className="form-grid" onSubmit={(event) => void create(event)}><label><span>Key name</span><input value={name} onChange={(event) => setName(event.target.value)} placeholder="Production SDK" maxLength={160} required disabled={!canManage} /></label><label><span>Data source</span><select value={sourceId} onChange={(event) => setSourceId(event.target.value)} required disabled={!canManage || !environment.data_sources.length}>{environment.data_sources.map((source) => <option value={source.id} key={source.id}>{source.name}</option>)}</select></label><label><span>Expires (optional)</span><input type="datetime-local" value={expires} onChange={(event) => setExpires(event.target.value)} disabled={!canManage} /></label><fieldset><legend>Ingestion scopes</legend><label className="check-label"><input type="checkbox" checked={otlp} onChange={(event) => setOtlp(event.target.checked)} disabled={!canManage} /> OTLP telemetry</label><label className="check-label"><input type="checkbox" checked={replay} onChange={(event) => setReplay(event.target.checked)} disabled={!canManage} /> Session replay</label></fieldset><button className="button primary form-submit" type="submit" disabled={!canManage || !sourceId || (!otlp && !replay) || !!pending}>{pending === "Issuing SDK key" ? "Issuing…" : "Issue key"}</button></form></section>
  </div>;
}

function SchemasPage({ api, project, canWrite, pending, runAction }: { api: ChillApi; project: ConsoleProject; canWrite: boolean; pending: string; runAction: ActionRunner }) {
  const [version, setVersion] = useState(""); const [url, setUrl] = useState(""); const [compatibility, setCompatibility] = useState("exact");
  const [definition, setDefinition] = useState('{\n  "type": "object",\n  "properties": {}\n}');
  const submit = async (event: FormEvent) => { event.preventDefault(); try { const parsed = parseJsonObject(definition, "Schema definition"); await runAction("Registering schema", async () => { await api.createSchema({ project_id: project.id, version, url, definition: parsed, compatibility }); }, `Schema ${version} is registered.`); setVersion(""); setUrl(""); } catch (error) { if (error instanceof Error && error.message.includes("JSON")) window.alert(error.message); } };
  return <div className="page"><PageHeading eyebrow="Contract" title="Schema catalog" detail="Version behavior payloads explicitly so SDKs, ingestion, and query tools share one durable contract." />
    <section className="panel"><div className="panel-heading"><div><h2>Registered schemas</h2><p>{project.schemas.length} version{project.schemas.length === 1 ? "" : "s"} for {project.name}</p></div></div>{project.schemas.length ? <div className="schema-grid">{project.schemas.map((schema) => <article className="schema-card" key={schema.id}><div><span className="schema-version">v{schema.version}</span><span className={`state-label ${schema.status}`}>{schema.status}</span></div><strong>{schema.url}</strong><dl><div><dt>Compatibility</dt><dd>{schema.compatibility}</dd></div><div><dt>Properties</dt><dd>{Object.keys((schema.definition.properties as Record<string, unknown> | undefined) ?? {}).length}</dd></div></dl></article>)}</div> : <EmptyInline icon="schema" title="No behavior schemas" detail="Register the first semantic version and JSON Schema definition." />}</section>
    <section className="panel form-panel"><div className="panel-heading"><div><h2>Register a schema</h2><p>Versions and canonical URLs must be unique within the project.</p></div></div><PermissionNote capability="control:write" allowed={canWrite} /><form className="schema-form" onSubmit={(event) => void submit(event)}><div className="form-grid compact"><label><span>Semantic version</span><input value={version} onChange={(event) => setVersion(event.target.value)} placeholder="1.0.0" pattern="(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)" required disabled={!canWrite} /></label><label><span>Canonical URL</span><input type="url" value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://schemas.example.com/behavior/v1.json" required disabled={!canWrite} /></label><label><span>Compatibility</span><select value={compatibility} onChange={(event) => setCompatibility(event.target.value)} disabled={!canWrite}>{["exact", "backward", "forward", "full"].map((value) => <option key={value}>{value}</option>)}</select></label></div><label><span>JSON Schema definition</span><textarea className="code-editor" value={definition} onChange={(event) => setDefinition(event.target.value)} rows={12} spellCheck={false} required disabled={!canWrite} /></label><button className="button primary" type="submit" disabled={!canWrite || !!pending}>{pending === "Registering schema" ? "Registering…" : "Register schema"}</button></form></section>
  </div>;
}

function ControlsPage({ api, project, environment, canWrite, pending, runAction }: { api: ChillApi; project: ConsoleProject; environment: ConsoleEnvironment; canWrite: boolean; pending: string; runAction: ActionRunner }) {
  const sampling = environment.sampling;
  const [retention, setRetention] = useState(String(environment.retention_days));
  const [behaviorN, setBehaviorN] = useState(String(sampling?.behavior_numerator ?? 1)); const [behaviorD, setBehaviorD] = useState(String(sampling?.behavior_denominator ?? 1));
  const [replayN, setReplayN] = useState(String(sampling?.replay_numerator ?? 0)); const [replayD, setReplayD] = useState(String(sampling?.replay_denominator ?? 1)); const [salt, setSalt] = useState(sampling?.salt_version ?? "v1");
  const [privacy, setPrivacy] = useState(JSON.stringify(environment.privacy?.document ?? defaultPrivacyPolicy(), null, 2));
  const updateRetention = async (event: FormEvent) => { event.preventDefault(); try { await runAction("Updating retention", () => api.updateRetention(environment.id, Number(retention)), `Retention is now ${retention} days.`); } catch { /* surfaced */ } };
  const updateSampling = async (event: FormEvent) => { event.preventDefault(); try { await runAction("Activating sampling", () => api.activateSampling({ project_id: project.id, environment_id: environment.id, behavior_numerator: Number(behaviorN), behavior_denominator: Number(behaviorD), replay_numerator: Number(replayN), replay_denominator: Number(replayD), salt_version: salt }), "The new sampling policy is active."); } catch { /* surfaced */ } };
  const updatePrivacy = async (event: FormEvent) => { event.preventDefault(); try { const document = parseJsonObject(privacy, "Privacy policy"); await runAction("Activating privacy policy", () => api.activatePrivacy({ project_id: project.id, environment_id: environment.id, document }), "The new privacy policy is active."); } catch (error) { if (error instanceof Error && error.message.includes("JSON")) window.alert(error.message); } };
  return <div className="page"><PageHeading eyebrow="Governance" title="Collection controls" detail="Every change creates a new audited policy version. Local consent can only narrow these server-side settings." />
    <PermissionNote capability="control:write" allowed={canWrite} />
    <div className="controls-grid"><section className="panel control-card"><div className="control-heading"><span className="control-icon"><Icon name="clock" /></span><div><h2>Retention</h2><p>Behavior data lifetime for {environment.name}.</p></div><span className="version-chip">Current</span></div><form onSubmit={(event) => void updateRetention(event)}><label><span>Retention days</span><div className="input-suffix"><input type="number" min={1} max={3650} value={retention} onChange={(event) => setRetention(event.target.value)} disabled={!canWrite} /><span>days</span></div></label><button className="button secondary" type="submit" disabled={!canWrite || !!pending || Number(retention) === environment.retention_days}>{pending === "Updating retention" ? "Saving…" : "Save retention"}</button></form></section>
      <section className="panel control-card sampling-card"><div className="control-heading"><span className="control-icon"><Icon name="sampling" /></span><div><h2>Sampling</h2><p>Deterministic behavior and replay rates.</p></div>{sampling && <span className="version-chip">v{sampling.version}</span>}</div><form onSubmit={(event) => void updateSampling(event)}><div className="ratio-row"><label><span>Behavior sample</span><div><input type="number" min={0} value={behaviorN} onChange={(event) => setBehaviorN(event.target.value)} disabled={!canWrite} /><span>/</span><input type="number" min={1} value={behaviorD} onChange={(event) => setBehaviorD(event.target.value)} disabled={!canWrite} /></div><small>{samplingPercentage(Number(behaviorN), Number(behaviorD))}</small></label><label><span>Replay sample</span><div><input type="number" min={0} value={replayN} onChange={(event) => setReplayN(event.target.value)} disabled={!canWrite} /><span>/</span><input type="number" min={1} value={replayD} onChange={(event) => setReplayD(event.target.value)} disabled={!canWrite} /></div><small>{samplingPercentage(Number(replayN), Number(replayD))}</small></label></div><label><span>Salt version</span><input value={salt} onChange={(event) => setSalt(event.target.value)} maxLength={64} required disabled={!canWrite} /></label><button className="button secondary" type="submit" disabled={!canWrite || !!pending}>{pending === "Activating sampling" ? "Activating…" : "Activate sampling version"}</button></form></section>
    </div>
    <section className="panel form-panel privacy-panel"><div className="panel-heading"><div><h2>Privacy policy</h2><p>Fail-closed capture rules. Source masking cannot be relaxed after collection.</p></div>{environment.privacy && <span className="version-chip">v{environment.privacy.version}</span>}</div><form onSubmit={(event) => void updatePrivacy(event)}><label><span>Validated policy document</span><textarea className="code-editor privacy-editor" rows={20} value={privacy} onChange={(event) => setPrivacy(event.target.value)} spellCheck={false} disabled={!canWrite} /></label><div className="policy-warning"><Icon name="shield" /><div><strong>Activation is immediate</strong><p>SDKs receive the new effective policy on their next collection-state refresh.</p></div></div><button className="button primary" type="submit" disabled={!canWrite || !!pending}>{pending === "Activating privacy policy" ? "Validating and activating…" : "Validate and activate policy"}</button></form></section>
  </div>;
}

function SecretReveal({ secret, onClose }: { secret: Secret; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => { try { await navigator.clipboard.writeText(secret.credential); setCopied(true); } catch { window.prompt("Copy the credential", secret.credential); } };
  return <div className="modal-backdrop" role="presentation"><section className="secret-modal" role="dialog" aria-modal="true" aria-labelledby="secret-title"><span className="secret-icon"><Icon name="key" /></span><span className="eyebrow">One-time disclosure</span><h2 id="secret-title">{secret.title}</h2><p>Copy this credential now. Chill stores only its non-secret prefix and digest, so it cannot be shown again.</p><div className="secret-value"><code>{secret.credential}</code><button type="button" onClick={() => void copy()}><Icon name="copy" /> {copied ? "Copied" : "Copy"}</button></div><div className="secret-preview">Verify: <span className="mono">{credentialPreview(secret.credential)}</span></div><button className="button primary large" type="button" onClick={onClose}>I saved the credential</button></section></div>;
}

function PermissionNote({ capability, allowed }: { capability: Capability; allowed: boolean }) {
  if (allowed) return null;
  return <div className="permission-note"><Icon name="lock" /><span>Read-only view. Your current role does not grant <code>{capability}</code>.</span></div>;
}

function NoticeBanner({ notice, onDismiss }: { notice: Notice; onDismiss: () => void }) {
  return <div className={`notice ${notice.kind}`} role={notice.kind === "error" ? "alert" : "status"}><Icon name={notice.kind === "error" ? "alert" : "check"} /><span>{notice.message}</span><button type="button" onClick={onDismiss} aria-label="Dismiss notification">×</button></div>;
}

function EmptyState({ title, detail }: { title: string; detail: string }) { return <section className="empty-state"><span><Icon name="project" /></span><h1>{title}</h1><p>{detail}</p></section>; }
function EmptyInline({ icon, title, detail }: { icon: IconName; title: string; detail: string }) { return <div className="empty-inline"><span><Icon name={icon} /></span><div><strong>{title}</strong><p>{detail}</p></div></div>; }

type IconName = "alert" | "arrow" | "chart" | "check" | "clock" | "copy" | "home" | "key" | "lock" | "logout" | "project" | "refresh" | "sampling" | "schema" | "shield" | "source";
function Icon({ name }: { name: IconName }) {
  const paths: Record<IconName, ReactNode> = {
    alert: <><path d="M12 9v4"/><path d="M12 17h.01"/><path d="M10.3 3.8 2.2 18a2 2 0 0 0 1.7 3h16.2a2 2 0 0 0 1.7-3L13.7 3.8a2 2 0 0 0-3.4 0Z"/></>,
    arrow: <><path d="M5 12h14"/><path d="m13 6 6 6-6 6"/></>,
    chart: <><path d="M4 20V10M10 20V4M16 20v-7M22 20H2"/></>,
    check: <path d="m5 12 4 4L19 6"/>,
    clock: <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>,
    copy: <><rect x="8" y="8" width="11" height="11" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/></>,
    home: <><path d="m3 11 9-8 9 8"/><path d="M5 10v10h14V10"/><path d="M9 20v-6h6v6"/></>,
    key: <><circle cx="8" cy="15" r="4"/><path d="m11 12 8-8"/><path d="m15 8 3 3"/><path d="m17 6 2 2"/></>,
    lock: <><rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/></>,
    logout: <><path d="M10 17l5-5-5-5"/><path d="M15 12H3"/><path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"/></>,
    project: <><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 8h10M7 12h7M7 16h4"/></>,
    refresh: <><path d="M20 7v5h-5"/><path d="M4 17v-5h5"/><path d="M6.1 9A7 7 0 0 1 18 6l2 6M4 12l2 6a7 7 0 0 0 11.9-3"/></>,
    sampling: <><path d="M4 19V9M10 19V5M16 19v-7M22 19V3"/></>,
    schema: <><path d="M6 3h9l4 4v14H6z"/><path d="M14 3v5h5M9 12h7M9 16h7"/></>,
    shield: <><path d="M12 22s8-4 8-11V5l-8-3-8 3v6c0 7 8 11 8 11Z"/><path d="m9 12 2 2 4-5"/></>,
    source: <><circle cx="5" cy="12" r="2"/><circle cx="19" cy="5" r="2"/><circle cx="19" cy="19" r="2"/><path d="m7 11 10-5M7 13l10 5"/></>,
  };
  return <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[name]}</svg>;
}

function selectProject(overview: ConsoleOverview | null, preferred: string): ConsoleProject | null { return overview?.projects.find((project) => project.id === preferred) ?? overview?.projects[0] ?? null; }
function selectEnvironment(project: ConsoleProject | null, preferred: string): ConsoleEnvironment | null { return project?.environments.find((environment) => environment.id === preferred) ?? project?.environments[0] ?? null; }
function initials(name: string): string { return name.split(/\s+/).filter(Boolean).slice(0, 2).map((part) => part[0]?.toUpperCase()).join("") || "CH"; }
function sourceName(environment: ConsoleEnvironment, sourceId: string): string { return environment.data_sources.find((source) => source.id === sourceId)?.name ?? "Unknown source"; }
function labelKind(kind: string): string { return ({ apple: "Apple", android: "Android", web: "Web", server: "Server", otlp: "Generic OTLP" } as Record<string, string>)[kind] ?? kind; }
function formatDate(value?: string): string { if (!value) return "Never"; const date = new Date(value); return Number.isNaN(date.valueOf()) ? "Unknown" : new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(date); }
function messageFrom(error: unknown): string { return error instanceof Error ? error.message : "Something went wrong."; }
function canonicalEvent(eventClass: "lifecycle" | "domain" | "error", emission: "observed" | "succeeded" | "terminal" = "observed"): Readonly<Record<string, unknown>> { return { event_class: eventClass, emission }; }
function slugify(value: string): string { return value.toLowerCase().trim().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 63); }
function defaultPrivacyPolicy(): Record<string, unknown> { return { schema_version: "1.0.0", policy_version: "privacy-v1", default_disposition: "omit", annotation_allowlist: {}, source_allowlist: ["platform"], replay: { mask_at_source: true, text: "mask", form_values: "mask", accessibility_text: "mask", secure_input: "mask", pixels: "mask", custom_drawing: "block" } }; }
