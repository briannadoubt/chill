using System;
using System.Collections;
using System.Text;
using UnityEngine;
using UnityEngine.Networking;

namespace Chill.Unity
{
    public static class ChillRuntime
    {
        private static ChillClient client;
        private static ChillRuntimeHost host;

        public static bool IsConfigured
        {
            get { return client != null; }
        }

        public static IReadOnlyClient Client
        {
            get { return client == null ? null : new IReadOnlyClient(client); }
        }

        public static void Configure(ChillConfiguration configuration)
        {
            if (client != null)
            {
                throw new InvalidOperationException("ChillRuntime is already configured.");
            }
            client = new ChillClient(configuration);
            client.Start();
            var gameObject = new GameObject("[Chill Runtime]");
            gameObject.hideFlags = HideFlags.HideInHierarchy;
            UnityEngine.Object.DontDestroyOnLoad(gameObject);
            host = gameObject.AddComponent<ChillRuntimeHost>();
            host.Initialize(client);
        }

        public static void SetConsent(ChillConsent consent)
        {
            RequireClient();
            if (consent == ChillConsent.Denied && host != null)
            {
                host.AbortFlush();
            }
            client.SetConsent(consent);
        }

        public static void DeclareAnnotation(
            string key,
            ChillAnnotationClassification classification = ChillAnnotationClassification.Internal)
        {
            RequireClient();
            client.DeclareAnnotation(key, classification);
        }

        public static bool Action(
            string name,
            ChillAnnotations annotations = null,
            ChillActionActivation activation = ChillActionActivation.Primary,
            ChillInput input = ChillInput.Unknown,
            string role = "control")
        {
            return client != null && client.Action(name, annotations, activation, input, role);
        }

        public static bool Event(
            string name,
            ChillAnnotations annotations = null,
            ChillEventClass eventClass = ChillEventClass.Domain,
            ChillEventSeverity severity = ChillEventSeverity.Info)
        {
            return client != null && client.Event(name, annotations, eventClass, severity);
        }

        public static bool Impression(
            string name,
            ChillAnnotations annotations = null,
            string role = "content",
            double visibilityRatio = 1d)
        {
            return client != null && client.Impression(name, annotations, role, visibilityRatio);
        }

        public static ChillActivityScope BeginActivity(
            string name,
            ChillAnnotations annotations = null,
            ChillActivityKind kind = ChillActivityKind.Domain)
        {
            return client == null
                ? new ChillActivityScope(null, null, null, null, null, 0L, kind)
                : client.BeginActivity(name, annotations, kind);
        }

        public static ChillPageScope StartPage(
            string segment,
            ChillAnnotations annotations = null,
            ChillPageRelation relation = ChillPageRelation.Root,
            ChillPageCause cause = ChillPageCause.Navigate,
            bool inheritCurrentPath = false)
        {
            return client == null
                ? new ChillPageScope(null, null)
                : client.StartPage(segment, annotations, relation, cause, inheritCurrentPath);
        }

        public static void Flush()
        {
            if (host != null)
            {
                host.Flush();
            }
        }

        public static void Shutdown()
        {
            if (client == null)
            {
                return;
            }
            client.Stop();
            if (host != null)
            {
                host.Detach();
                UnityEngine.Object.Destroy(host.gameObject);
            }
            client.Dispose();
            client = null;
            host = null;
        }

        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
        private static void ResetStatics()
        {
            client = null;
            host = null;
        }

        private static void RequireClient()
        {
            if (client == null)
            {
                throw new InvalidOperationException("Call ChillRuntime.Configure before using Chill.");
            }
        }

        public sealed class IReadOnlyClient
        {
            private readonly ChillClient value;

            internal IReadOnlyClient(ChillClient value)
            {
                this.value = value;
            }

            public ChillConsent Consent
            {
                get { return value.Consent; }
            }

            public System.Collections.Generic.IReadOnlyList<ChillDiagnostic> Diagnostics
            {
                get { return value.Diagnostics; }
            }
        }
    }

    internal sealed class ChillRuntimeHost : MonoBehaviour
    {
        private ChillClient client;
        private float nextFlush;
        private bool flushing;
        private UnityWebRequest currentRequest;

        internal void Initialize(ChillClient value)
        {
            client = value;
            nextFlush = Time.realtimeSinceStartup + client.FlushIntervalSeconds;
            Application.logMessageReceivedThreaded += OnLogMessage;
        }

        internal void Flush()
        {
            if (!flushing && client != null && client.Consent == ChillConsent.Granted)
            {
                StartCoroutine(FlushCoroutine());
            }
        }

        internal void AbortFlush()
        {
            if (currentRequest != null)
            {
                currentRequest.Abort();
            }
        }

        internal void Detach()
        {
            Application.logMessageReceivedThreaded -= OnLogMessage;
            AbortFlush();
            StopAllCoroutines();
            if (currentRequest != null)
            {
                currentRequest.Dispose();
                currentRequest = null;
            }
            flushing = false;
            client = null;
        }

        private void Update()
        {
            if (client != null && Time.realtimeSinceStartup >= nextFlush)
            {
                nextFlush = Time.realtimeSinceStartup + client.FlushIntervalSeconds;
                Flush();
            }
        }

        private void OnApplicationPause(bool paused)
        {
            if (client == null)
            {
                return;
            }
            if (paused)
            {
                client.Event(
                    "app.background",
                    null,
                    ChillEventClass.Lifecycle,
                    ChillEventSeverity.Info);
                Flush();
            }
            else
            {
                client.Event(
                    "app.foreground",
                    null,
                    ChillEventClass.Lifecycle,
                    ChillEventSeverity.Info);
            }
        }

        private void OnApplicationQuit()
        {
            if (client != null)
            {
                client.Stop();
            }
        }

        private void OnDestroy()
        {
            Detach();
        }

        private void OnLogMessage(string condition, string stackTrace, LogType type)
        {
            if (type == LogType.Exception && client != null)
            {
                client.Event(
                    "app.unhandled_exception",
                    null,
                    ChillEventClass.Crash,
                    ChillEventSeverity.Fatal);
            }
        }

        private IEnumerator FlushCoroutine()
        {
            flushing = true;
            ChillExportBatch batch = client.CreateExportBatch();
            if (batch == null)
            {
                flushing = false;
                yield break;
            }

            byte[] body = Encoding.UTF8.GetBytes(batch.Body);
            currentRequest = new UnityWebRequest(client.Endpoint.AbsoluteUri, UnityWebRequest.kHttpVerbPOST);
            currentRequest.uploadHandler = new UploadHandlerRaw(body);
            currentRequest.downloadHandler = new DownloadHandlerBuffer();
            currentRequest.SetRequestHeader("Authorization", "Bearer " + client.SdkKey);
            currentRequest.SetRequestHeader("Content-Type", "application/json");
            currentRequest.SetRequestHeader("X-Chill-Schema-Version", "1.0.0");

            yield return currentRequest.SendWebRequest();

            bool success = currentRequest.result == UnityWebRequest.Result.Success
                && currentRequest.responseCode >= 200
                && currentRequest.responseCode < 300;
            currentRequest.Dispose();
            currentRequest = null;
            if (success)
            {
                client.Acknowledge(batch);
            }
            else
            {
                client.ReportTransportFailure();
            }
            flushing = false;
        }
    }
}
