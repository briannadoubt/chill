using System;
using System.Collections.Generic;
using System.IO;
using UnityEngine;

namespace Chill.Unity
{
    public sealed class ChillConfiguration
    {
        private readonly Dictionary<string, ChillAnnotationClassification> annotationDefinitions =
            new Dictionary<string, ChillAnnotationClassification>(StringComparer.Ordinal);

        public ChillConfiguration(string serviceName, Uri endpoint, string sdkKey)
        {
            if (!ChillNames.IsSemantic(serviceName))
            {
                throw new ArgumentException("Service name must be a lowercase semantic identifier.", "serviceName");
            }
            ValidateEndpoint(endpoint);
            if (string.IsNullOrEmpty(sdkKey) || sdkKey.Length > 512)
            {
                throw new ArgumentException("An SDK key containing at most 512 characters is required.", "sdkKey");
            }

            ServiceName = serviceName;
            Endpoint = endpoint;
            SdkKey = sdkKey;
            Consent = ChillConsent.Denied;
            PolicyVersion = "privacy-v1";
            QueueDirectory = Path.Combine(Application.persistentDataPath, "Chill", "Queue");
            MaximumQueueBytes = 64L * 1024L * 1024L;
            MaximumBatchRecords = 100;
            FlushIntervalSeconds = 10f;
            SampleRate = 1d;
        }

        public string ServiceName { get; private set; }
        public Uri Endpoint { get; private set; }
        public string SdkKey { get; private set; }
        public ChillConsent Consent { get; set; }
        public string PolicyVersion { get; set; }
        public string QueueDirectory { get; set; }
        public long MaximumQueueBytes { get; set; }
        public int MaximumBatchRecords { get; set; }
        public float FlushIntervalSeconds { get; set; }
        public double SampleRate { get; set; }
        public string InstallationId { get; set; }

        internal IReadOnlyDictionary<string, ChillAnnotationClassification> AnnotationDefinitions
        {
            get { return annotationDefinitions; }
        }

        public ChillConfiguration AllowAnnotation(
            string key,
            ChillAnnotationClassification classification = ChillAnnotationClassification.Internal)
        {
            if (!ChillNames.IsAnnotationKey(key))
            {
                throw new ArgumentException("Annotation keys must be lowercase dotted identifiers.", "key");
            }
            annotationDefinitions[key] = classification;
            return this;
        }

        internal void Validate()
        {
            ValidateEndpoint(Endpoint);
            if (string.IsNullOrEmpty(PolicyVersion) || PolicyVersion.Length > 64)
            {
                throw new ArgumentException("PolicyVersion must contain 1 to 64 characters.");
            }
            if (string.IsNullOrEmpty(QueueDirectory))
            {
                throw new ArgumentException("QueueDirectory is required.");
            }
            if (MaximumQueueBytes < 1024 || MaximumQueueBytes > 1024L * 1024L * 1024L)
            {
                throw new ArgumentOutOfRangeException("MaximumQueueBytes");
            }
            if (MaximumBatchRecords < 1 || MaximumBatchRecords > 1000)
            {
                throw new ArgumentOutOfRangeException("MaximumBatchRecords");
            }
            if (FlushIntervalSeconds < 1f || FlushIntervalSeconds > 3600f)
            {
                throw new ArgumentOutOfRangeException("FlushIntervalSeconds");
            }
            if (SampleRate < 0d || SampleRate > 1d)
            {
                throw new ArgumentOutOfRangeException("SampleRate");
            }
            if (!string.IsNullOrEmpty(InstallationId) && !ChillIds.IsUuidV4(InstallationId))
            {
                throw new ArgumentException("InstallationId must be a lowercase UUIDv4.");
            }
        }

        private static void ValidateEndpoint(Uri endpoint)
        {
            if (endpoint == null || !endpoint.IsAbsoluteUri)
            {
                throw new ArgumentException("Endpoint must be an absolute URI.", "endpoint");
            }
            bool loopback = endpoint.IsLoopback;
            if (!string.Equals(endpoint.Scheme, Uri.UriSchemeHttps, StringComparison.OrdinalIgnoreCase)
                && !(loopback && string.Equals(endpoint.Scheme, Uri.UriSchemeHttp, StringComparison.OrdinalIgnoreCase)))
            {
                throw new ArgumentException("Endpoint must use HTTPS, except for loopback development.", "endpoint");
            }
            if (!string.IsNullOrEmpty(endpoint.UserInfo)
                || !string.Equals(endpoint.AbsolutePath, "/v1/logs", StringComparison.Ordinal)
                || !string.IsNullOrEmpty(endpoint.Query)
                || !string.IsNullOrEmpty(endpoint.Fragment))
            {
                throw new ArgumentException("Endpoint must be an exact /v1/logs URL without credentials, query, or fragment.", "endpoint");
            }
        }
    }
}
