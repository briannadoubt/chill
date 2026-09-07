using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using UnityEngine;

namespace Chill.Unity
{
    public sealed class ChillClient : IDisposable
    {
        private const long MaximumSafeSequence = 9007199254740991L;
        private readonly object gate = new object();
        private readonly ChillConfiguration configuration;
        private readonly IChillClock clock;
        private readonly Dictionary<string, ChillAnnotationClassification> annotationDefinitions;
        private readonly List<ChillDiagnostic> diagnostics = new List<ChillDiagnostic>();
        private readonly List<ChillPageState> pages = new List<ChillPageState>();
        private readonly string sourcePlatform;
        private readonly string unityVersion;
        private readonly string operatingSystem;
        private IChillStore store;
        private readonly IChillStore injectedStore;
        private bool sampled;
        private bool sampleDecisionMade;
        private bool started;
        private bool disposed;
        private ChillConsent consent;
        private long sequence;
        private string installationId;
        private string bootId;
        private string processId;
        private string sessionId;

        public ChillClient(ChillConfiguration configuration)
            : this(
                configuration,
                new ChillSystemClock(),
                null,
                ChillUnityPlatform.SourcePlatform(Application.platform),
                Application.unityVersion,
                SystemInfo.operatingSystemFamily.ToString().ToLowerInvariant())
        {
        }

        internal ChillClient(
            ChillConfiguration configuration,
            IChillClock clock,
            IChillStore store,
            string sourcePlatform,
            string unityVersion,
            string operatingSystem)
        {
            if (configuration == null)
            {
                throw new ArgumentNullException("configuration");
            }
            configuration.Validate();
            this.configuration = configuration;
            this.clock = clock;
            injectedStore = store;
            this.sourcePlatform = sourcePlatform;
            this.unityVersion = unityVersion;
            this.operatingSystem = operatingSystem;
            consent = configuration.Consent;
            annotationDefinitions =
                new Dictionary<string, ChillAnnotationClassification>(
                    configuration.AnnotationDefinitions,
                    StringComparer.Ordinal);
        }

        public bool IsStarted
        {
            get
            {
                lock (gate)
                {
                    return started;
                }
            }
        }

        public ChillConsent Consent
        {
            get
            {
                lock (gate)
                {
                    return consent;
                }
            }
        }

        public IReadOnlyList<ChillDiagnostic> Diagnostics
        {
            get
            {
                lock (gate)
                {
                    return diagnostics.ToArray();
                }
            }
        }

        internal Uri Endpoint
        {
            get { return configuration.Endpoint; }
        }

        internal string SdkKey
        {
            get { return configuration.SdkKey; }
        }

        internal float FlushIntervalSeconds
        {
            get { return configuration.FlushIntervalSeconds; }
        }

        public void Start()
        {
            lock (gate)
            {
                ThrowIfDisposed();
                if (started)
                {
                    return;
                }
                started = true;
                if (CanCapture())
                {
                    OpenSession();
                }
            }
        }

        public void SetConsent(ChillConsent value)
        {
            lock (gate)
            {
                ThrowIfDisposed();
                if (consent == value)
                {
                    return;
                }
                consent = value;
                if (value == ChillConsent.Denied)
                {
                    sessionId = null;
                    pages.Clear();
                    if (store != null)
                    {
                        store.Purge();
                    }
                    return;
                }
                if (started && CanCapture())
                {
                    OpenSession();
                }
            }
        }

        public void DeclareAnnotation(
            string key,
            ChillAnnotationClassification classification = ChillAnnotationClassification.Internal)
        {
            if (!ChillNames.IsAnnotationKey(key))
            {
                throw new ArgumentException("Annotation keys must be lowercase dotted identifiers.", "key");
            }
            lock (gate)
            {
                ThrowIfDisposed();
                ChillAnnotationClassification existing;
                if (annotationDefinitions.TryGetValue(key, out existing) && existing != classification)
                {
                    Diagnostic("annotation_classification_conflict", key);
                    return;
                }
                annotationDefinitions[key] = classification;
            }
        }

        public bool Action(
            string name,
            ChillAnnotations annotations = null,
            ChillActionActivation activation = ChillActionActivation.Primary,
            ChillInput input = ChillInput.Unknown,
            string role = "control")
        {
            if (!ChillNames.IsSemantic(role))
            {
                throw new ArgumentException("Action role must be a semantic identifier.", "role");
            }
            var payload = new List<ChillOtlpAttribute>
            {
                ChillOtlp.Text("chill.payload.element_id", name),
                ChillOtlp.Text("chill.payload.role", role),
                ChillOtlp.Text("chill.payload.activation", ChillNames.SnakeCase(activation)),
                ChillOtlp.Text("chill.payload.input", ChillNames.SnakeCase(input))
            };
            lock (gate)
            {
                return Emit("action", "instant", name, null, annotations, payload, null, null, null);
            }
        }

        public bool Event(
            string name,
            ChillAnnotations annotations = null,
            ChillEventClass eventClass = ChillEventClass.Domain,
            ChillEventSeverity severity = ChillEventSeverity.Info)
        {
            var payload = new List<ChillOtlpAttribute>
            {
                ChillOtlp.Text("chill.payload.event_class", ChillNames.SnakeCase(eventClass)),
                ChillOtlp.Text("chill.payload.severity", ChillNames.SnakeCase(severity)),
                ChillOtlp.Text("chill.payload.emission", "observed")
            };
            lock (gate)
            {
                return Emit("event", "instant", name, null, annotations, payload, null, null, null);
            }
        }

        public bool Impression(
            string name,
            ChillAnnotations annotations = null,
            string role = "content",
            double visibilityRatio = 1d)
        {
            if (!ChillNames.IsSemantic(role))
            {
                throw new ArgumentException("Impression role must be a semantic identifier.", "role");
            }
            if (visibilityRatio < 0d || visibilityRatio > 1d)
            {
                throw new ArgumentOutOfRangeException("visibilityRatio");
            }
            var payload = new List<ChillOtlpAttribute>
            {
                ChillOtlp.Text("chill.payload.element_id", name),
                ChillOtlp.Text("chill.payload.role", role),
                ChillOtlp.Number("chill.payload.visibility_ratio", visibilityRatio)
            };
            lock (gate)
            {
                return Emit("impression", "instant", name, null, annotations, payload, null, null, null);
            }
        }

        public ChillActivityScope BeginActivity(
            string name,
            ChillAnnotations annotations = null,
            ChillActivityKind kind = ChillActivityKind.Domain)
        {
            if (!ChillNames.IsSemantic(name))
            {
                throw new ArgumentException("Activity name must be a semantic identifier.", "name");
            }
            lock (gate)
            {
                ThrowIfDisposed();
                if (!CanRecord())
                {
                    return new ChillActivityScope(null, null, null, null, null, 0L, kind);
                }
                string subjectId = ChillIds.NewUuidV7(clock.UnixMilliseconds);
                string traceparent = ChillIds.NewTraceparent();
                ChillPageState page = CurrentPage();
                var payload = new List<ChillOtlpAttribute>
                {
                    ChillOtlp.Text("chill.payload.activity_kind", ChillNames.SnakeCase(kind)),
                    ChillOtlp.Text("chill.payload.role", "operation"),
                    ChillOtlp.Integer("chill.payload.attempt", 1),
                    ChillOtlp.Integer("chill.payload.recursion_depth", 0)
                };
                bool emitted = Emit(
                    "activity",
                    "start",
                    name,
                    subjectId,
                    annotations,
                    payload,
                    null,
                    traceparent,
                    page);
                return emitted
                    ? new ChillActivityScope(this, name, subjectId, traceparent, annotations, Stopwatch.GetTimestamp(), kind, page)
                    : new ChillActivityScope(null, null, null, null, null, 0L, kind);
            }
        }

        public ChillPageScope StartPage(
            string segment,
            ChillAnnotations annotations = null,
            ChillPageRelation relation = ChillPageRelation.Root,
            ChillPageCause cause = ChillPageCause.Navigate,
            bool inheritCurrentPath = false)
        {
            if (!ChillNames.IsSemantic(segment))
            {
                throw new ArgumentException("Page segment must be a semantic identifier.", "segment");
            }
            lock (gate)
            {
                ThrowIfDisposed();
                if (!CanRecord())
                {
                    return new ChillPageScope(null, null);
                }

                ChillPageState parent = inheritCurrentPath ? CurrentPage() : null;
                string instanceId = ChillIds.NewUuidV7(clock.UnixMilliseconds);
                var path = new List<string>();
                var pathInstanceIds = new List<string>();
                if (parent != null)
                {
                    path.AddRange(parent.Path);
                    pathInstanceIds.AddRange(parent.PathInstanceIds);
                }
                if (path.Count >= 32)
                {
                    Diagnostic("page_depth_exceeded", segment);
                    return new ChillPageScope(null, null);
                }
                path.Add(segment);
                pathInstanceIds.Add(instanceId);
                var page = new ChillPageState(
                    parent == null ? instanceId : parent.SurfaceId,
                    instanceId,
                    path,
                    pathInstanceIds,
                    segment,
                    relation,
                    annotations);
                pages.Add(page);
                var payload = PagePayload(page, "visible", true, cause);
                bool emitted = Emit(
                    "page",
                    "start",
                    segment,
                    instanceId,
                    annotations,
                    payload,
                    null,
                    null,
                    page);
                if (!emitted)
                {
                    pages.Remove(page);
                    return new ChillPageScope(null, null);
                }
                return new ChillPageScope(this, page);
            }
        }

        public void Stop()
        {
            lock (gate)
            {
                if (disposed || !started)
                {
                    return;
                }
                for (int index = pages.Count - 1; index >= 0; index -= 1)
                {
                    EndPage(pages[index], ChillPageCause.SurfaceDestroyed);
                }
                if (CanRecord() && !string.IsNullOrEmpty(sessionId))
                {
                    var payload = new List<ChillOtlpAttribute>
                    {
                        ChillOtlp.Text("chill.payload.end_reason", "terminated")
                    };
                    Emit("session", "end", "app.session", sessionId, null, payload, ChillOutcome.Ok, null, null);
                }
                sessionId = null;
                started = false;
            }
        }

        public void Dispose()
        {
            lock (gate)
            {
                if (disposed)
                {
                    return;
                }
                Stop();
                disposed = true;
            }
        }

        internal ChillExportBatch CreateExportBatch()
        {
            lock (gate)
            {
                if (!CanRecord() || store == null)
                {
                    return null;
                }
                IReadOnlyList<ChillQueuedRecord> records = store.Peek(configuration.MaximumBatchRecords);
                if (records.Count == 0)
                {
                    return null;
                }
                string body = ChillOtlp.Batch(
                    configuration,
                    sourcePlatform,
                    processId,
                    unityVersion,
                    operatingSystem,
                    records);
                return new ChillExportBatch(records, body);
            }
        }

        internal void Acknowledge(ChillExportBatch batch)
        {
            if (batch == null)
            {
                return;
            }
            lock (gate)
            {
                if (consent == ChillConsent.Granted && store != null)
                {
                    store.Acknowledge(batch.Records);
                }
            }
        }

        internal void ReportTransportFailure()
        {
            lock (gate)
            {
                Diagnostic("flush_failed", "transport");
            }
        }

        internal void EndActivity(
            string name,
            string subjectId,
            string traceparent,
            ChillAnnotations annotations,
            long startedAt,
            ChillActivityKind kind,
            ChillPageState page,
            ChillOutcome outcome,
            string reasonCode)
        {
            lock (gate)
            {
                if (disposed || !started)
                {
                    return;
                }
                long elapsedTicks = Math.Max(0L, Stopwatch.GetTimestamp() - startedAt);
                long durationNanoseconds = (long)(elapsedTicks * (1000000000d / Stopwatch.Frequency));
                var payload = new List<ChillOtlpAttribute>
                {
                    ChillOtlp.Text("chill.payload.activity_kind", ChillNames.SnakeCase(kind)),
                    ChillOtlp.Text("chill.payload.role", "operation"),
                    ChillOtlp.Integer("chill.payload.attempt", 1),
                    ChillOtlp.Integer("chill.payload.recursion_depth", 0),
                    ChillOtlp.Integer(
                        "chill.duration_nano",
                        durationNanoseconds.ToString(CultureInfo.InvariantCulture))
                };
                if (!string.IsNullOrEmpty(reasonCode))
                {
                    payload.Add(ChillOtlp.Text("chill.outcome.reason_code", reasonCode));
                }
                Emit(
                    "activity",
                    "end",
                    name,
                    subjectId,
                    annotations,
                    payload,
                    outcome,
                    traceparent,
                    page);
            }
        }

        internal void EndPage(ChillPageState page, ChillPageCause cause)
        {
            lock (gate)
            {
                if (disposed || page == null || !pages.Contains(page))
                {
                    return;
                }
                var payload = PagePayload(page, "retained", false, cause);
                Emit(
                    "page",
                    "end",
                    page.Segment,
                    page.InstanceId,
                    page.Annotations,
                    payload,
                    ChillOutcome.Ok,
                    null,
                    page);
                pages.Remove(page);
            }
        }

        private void OpenSession()
        {
            EnsureCaptureState();
            if (!string.IsNullOrEmpty(sessionId))
            {
                return;
            }
            sessionId = ChillIds.NewUuidV7(clock.UnixMilliseconds);
            Emit("session", "start", "app.session", sessionId, null, new List<ChillOtlpAttribute>(), null, null, null);
        }

        private void EnsureCaptureState()
        {
            if (store == null)
            {
                try
                {
                    store = injectedStore ?? new ChillFileStore(
                        configuration.QueueDirectory,
                        configuration.MaximumQueueBytes,
                        delegate(string code) { Diagnostic(code, "queue"); });
                }
                catch
                {
                    Diagnostic("storage_failed", "queue_initialization");
                    store = new ChillMemoryStore();
                }
            }
            if (string.IsNullOrEmpty(installationId))
            {
                installationId = injectedStore == null
                    ? ChillInstallation.LoadOrCreate(configuration.QueueDirectory, configuration.InstallationId)
                    : (configuration.InstallationId ?? ChillIds.NewUuidV4());
            }
            if (string.IsNullOrEmpty(processId))
            {
                processId = ChillIds.NewUuidV4();
            }
            if (string.IsNullOrEmpty(bootId))
            {
                bootId = ChillIds.NewUuidV4();
            }
        }

        private bool Emit(
            string kind,
            string operation,
            string name,
            string subjectId,
            ChillAnnotations annotations,
            List<ChillOtlpAttribute> payload,
            ChillOutcome? outcome,
            string traceparent,
            ChillPageState page)
        {
            ThrowIfDisposed();
            if (!ChillNames.IsSemantic(name))
            {
                Diagnostic("invalid_name", name ?? "null");
                return false;
            }
            if (!CanRecord() || sequence >= MaximumSafeSequence)
            {
                return false;
            }
            EnsureCaptureState();
            long nextSequence = sequence + 1;
            string recordId = ChillIds.NewUuidV7(clock.UnixMilliseconds);
            var record = new ChillRecord
            {
                RecordId = recordId,
                SubjectId = subjectId ?? recordId,
                Kind = kind,
                Operation = operation,
                Name = name,
                OccurredAtUnixNanoseconds = clock.WallUnixNanoseconds,
                MonotonicNanoseconds = clock.MonotonicNanoseconds,
                Sequence = nextSequence,
                SessionId = kind == "session" ? (subjectId ?? sessionId) : sessionId,
                Traceparent = traceparent ?? ChillIds.NewTraceparent()
            };
            List<ChillAnnotationEntry> kept = FilterAnnotations(annotations);
            List<ChillOtlpAttribute> attributes = ChillOtlp.BaseAttributes(
                configuration,
                record,
                sourcePlatform,
                installationId,
                kept,
                annotationDefinitions);
            attributes.Add(ChillOtlp.Integer("chill.clock.monotonic_nano", record.MonotonicNanoseconds));
            attributes.Add(ChillOtlp.Text("chill.clock.boot_id", bootId));
            attributes.Add(ChillOtlp.Text("chill.source.process_id", processId));
            AddPageContext(attributes, page ?? CurrentPage());
            if (outcome.HasValue)
            {
                attributes.Add(ChillOtlp.Text("chill.outcome.status", ChillNames.SnakeCase(outcome.Value)));
            }
            attributes.AddRange(payload);
            record.Attributes = attributes;
            string logJson = record.ToLogJson();
            bool stored = store.Append(new ChillQueuedRecord(
                record.RecordId,
                record.Sequence,
                record.OccurredAtUnixNanoseconds,
                logJson));
            if (stored)
            {
                sequence = nextSequence;
            }
            return stored;
        }

        private List<ChillAnnotationEntry> FilterAnnotations(ChillAnnotations annotations)
        {
            var kept = new List<ChillAnnotationEntry>();
            if (annotations == null)
            {
                return kept;
            }
            IReadOnlyList<ChillAnnotationEntry> values = annotations.Entries;
            for (int index = 0; index < values.Count && kept.Count < 128; index += 1)
            {
                ChillAnnotationEntry value = values[index];
                if (!annotationDefinitions.ContainsKey(value.Key))
                {
                    Diagnostic("annotation_disallowed", value.Key);
                    continue;
                }
                kept.Add(value);
            }
            if (values.Count > 128)
            {
                Diagnostic("annotation_limit", "128");
            }
            return kept;
        }

        private static void AddPageContext(List<ChillOtlpAttribute> attributes, ChillPageState page)
        {
            if (page == null)
            {
                return;
            }
            attributes.Add(ChillOtlp.Text("chill.context.page.surface_id", page.SurfaceId));
            attributes.Add(ChillOtlp.Text("chill.context.page.instance_id", page.InstanceId));
            attributes.Add(ChillOtlp.Strings("chill.context.page.path", page.Path));
            attributes.Add(ChillOtlp.Strings("chill.context.page.path_instance_ids", page.PathInstanceIds));
        }

        private static List<ChillOtlpAttribute> PagePayload(
            ChillPageState page,
            string exposure,
            bool focused,
            ChillPageCause cause)
        {
            return new List<ChillOtlpAttribute>
            {
                ChillOtlp.Text("chill.payload.surface_id", page.SurfaceId),
                ChillOtlp.Text("chill.payload.instance_id", page.InstanceId),
                ChillOtlp.Strings("chill.payload.path", page.Path),
                ChillOtlp.Strings("chill.payload.path_instance_ids", page.PathInstanceIds),
                ChillOtlp.Text("chill.payload.relation", ChillNames.SnakeCase(page.Relation)),
                ChillOtlp.Text("chill.payload.exposure", exposure),
                ChillOtlp.Boolean("chill.payload.focused", focused),
                ChillOtlp.Text("chill.payload.cause", ChillNames.SnakeCase(cause))
            };
        }

        private ChillPageState CurrentPage()
        {
            return pages.Count == 0 ? null : pages[pages.Count - 1];
        }

        private bool CanCapture()
        {
            if (consent != ChillConsent.Granted)
            {
                return false;
            }
            if (!sampleDecisionMade)
            {
                sampled = configuration.SampleRate >= 1d
                    || (configuration.SampleRate > 0d
                        && ChillIds.RandomUnit() < configuration.SampleRate);
                sampleDecisionMade = true;
            }
            return sampled;
        }

        private bool CanRecord()
        {
            return started && CanCapture() && !string.IsNullOrEmpty(sessionId);
        }

        private void Diagnostic(string code, string detail)
        {
            if (diagnostics.Count >= 64)
            {
                diagnostics.RemoveAt(0);
            }
            diagnostics.Add(new ChillDiagnostic(code, detail));
        }

        private void ThrowIfDisposed()
        {
            if (disposed)
            {
                throw new ObjectDisposedException("ChillClient");
            }
        }
    }

    internal sealed class ChillPageState
    {
        internal ChillPageState(
            string surfaceId,
            string instanceId,
            IReadOnlyList<string> path,
            IReadOnlyList<string> pathInstanceIds,
            string segment,
            ChillPageRelation relation,
            ChillAnnotations annotations)
        {
            SurfaceId = surfaceId;
            InstanceId = instanceId;
            Path = path;
            PathInstanceIds = pathInstanceIds;
            Segment = segment;
            Relation = relation;
            Annotations = annotations;
        }

        internal string SurfaceId { get; private set; }
        internal string InstanceId { get; private set; }
        internal IReadOnlyList<string> Path { get; private set; }
        internal IReadOnlyList<string> PathInstanceIds { get; private set; }
        internal string Segment { get; private set; }
        internal ChillPageRelation Relation { get; private set; }
        internal ChillAnnotations Annotations { get; private set; }
    }

    public sealed class ChillActivityScope : IDisposable
    {
        private ChillClient client;
        private readonly string name;
        private readonly string subjectId;
        private readonly string traceparent;
        private readonly ChillAnnotations annotations;
        private readonly long startedAt;
        private readonly ChillActivityKind kind;
        private readonly ChillPageState page;

        internal ChillActivityScope(
            ChillClient client,
            string name,
            string subjectId,
            string traceparent,
            ChillAnnotations annotations,
            long startedAt,
            ChillActivityKind kind,
            ChillPageState page = null)
        {
            this.client = client;
            this.name = name;
            this.subjectId = subjectId;
            this.traceparent = traceparent;
            this.annotations = annotations;
            this.startedAt = startedAt;
            this.kind = kind;
            this.page = page;
        }

        public void Succeed()
        {
            Finish(ChillOutcome.Ok, null);
        }

        public void Fail(string reasonCode = null)
        {
            Finish(ChillOutcome.Error, ValidateReason(reasonCode));
        }

        public void Cancel(string reasonCode = null)
        {
            Finish(ChillOutcome.Cancelled, ValidateReason(reasonCode));
        }

        public void Timeout(string reasonCode = null)
        {
            Finish(ChillOutcome.Timeout, ValidateReason(reasonCode));
        }

        public void Dispose()
        {
            Finish(ChillOutcome.Cancelled, "scope_disposed");
        }

        private void Finish(ChillOutcome outcome, string reasonCode)
        {
            ChillClient active = client;
            if (active == null)
            {
                return;
            }
            client = null;
            active.EndActivity(name, subjectId, traceparent, annotations, startedAt, kind, page, outcome, reasonCode);
        }

        private static string ValidateReason(string reasonCode)
        {
            if (string.IsNullOrEmpty(reasonCode))
            {
                return null;
            }
            if (!ChillNames.IsSemantic(reasonCode))
            {
                throw new ArgumentException("Reason codes must be semantic identifiers.", "reasonCode");
            }
            return reasonCode;
        }
    }

    public sealed class ChillPageScope : IDisposable
    {
        private ChillClient client;
        private readonly ChillPageState page;

        internal ChillPageScope(ChillClient client, ChillPageState page)
        {
            this.client = client;
            this.page = page;
        }

        public void Dispose()
        {
            ChillClient active = client;
            if (active == null)
            {
                return;
            }
            client = null;
            active.EndPage(page, ChillPageCause.Dismiss);
        }
    }

    internal static class ChillUnityPlatform
    {
        internal static string SourcePlatform(RuntimePlatform platform)
        {
            switch (platform)
            {
                case RuntimePlatform.Android:
                    return "android";
                case RuntimePlatform.IPhonePlayer:
                case RuntimePlatform.tvOS:
                    return "apple";
                case RuntimePlatform.WebGLPlayer:
                    return "web";
                default:
                    return "server";
            }
        }
    }
}
