using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Chill.Unity
{
    internal sealed class ChillOtlpAttribute
    {
        internal ChillOtlpAttribute(string key, string valueJson)
        {
            Key = key;
            ValueJson = valueJson;
        }

        internal string Key { get; private set; }
        internal string ValueJson { get; private set; }

        internal string Json
        {
            get { return ChillJson.Attribute(Key, ValueJson); }
        }
    }

    internal sealed class ChillRecord
    {
        internal string RecordId;
        internal string SubjectId;
        internal string Kind;
        internal string Operation;
        internal string Name;
        internal string OccurredAtUnixNanoseconds;
        internal string MonotonicNanoseconds;
        internal long Sequence;
        internal string SessionId;
        internal string Traceparent;
        internal IReadOnlyList<ChillOtlpAttribute> Attributes;

        internal string ToLogJson()
        {
            var all = new List<string>();
            for (int index = 0; index < Attributes.Count; index += 1)
            {
                all.Add(Attributes[index].Json);
            }

            var result = new StringBuilder("{\"timeUnixNano\":");
            result.Append(ChillJson.Quote(OccurredAtUnixNanoseconds));
            result.Append(",\"observedTimeUnixNano\":");
            result.Append(ChillJson.Quote(OccurredAtUnixNanoseconds));
            result.Append(",\"eventName\":");
            result.Append(ChillJson.Quote(Name));
            if (!string.IsNullOrEmpty(Traceparent))
            {
                result.Append(",\"traceId\":");
                result.Append(ChillJson.Quote(ChillIds.TraceId(Traceparent)));
                result.Append(",\"spanId\":");
                result.Append(ChillJson.Quote(ChillIds.SpanId(Traceparent)));
                result.Append(",\"flags\":1");
            }
            result.Append(",\"attributes\":");
            result.Append(ChillJson.Array(all));
            result.Append('}');
            return result.ToString();
        }
    }

    internal sealed class ChillExportBatch
    {
        internal ChillExportBatch(IReadOnlyList<ChillQueuedRecord> records, string body)
        {
            Records = records;
            Body = body;
        }

        internal IReadOnlyList<ChillQueuedRecord> Records { get; private set; }
        internal string Body { get; private set; }
    }

    internal static class ChillOtlp
    {
        private const string Schema = "https://schemas.chill.dev/behavior/v1/envelope.schema.json";

        internal static List<ChillOtlpAttribute> BaseAttributes(
            ChillConfiguration configuration,
            ChillRecord record,
            string sourcePlatform,
            string installationId,
            IReadOnlyList<ChillAnnotationEntry> annotations,
            IReadOnlyDictionary<string, ChillAnnotationClassification> definitions)
        {
            var attributes = new List<ChillOtlpAttribute>
            {
                Text("chill.schema.version", "1.0.0"),
                Text("chill.schema.url", Schema),
                Text("chill.record.id", record.RecordId),
                Text("chill.subject.id", record.SubjectId),
                Text("chill.behavior.kind", record.Kind),
                Text("chill.behavior.operation", record.Operation),
                Text("chill.behavior.name", record.Name),
                Integer("chill.clock.sequence_number", record.Sequence),
                Text("chill.source.platform", sourcePlatform),
                Text("chill.source.installation_id", installationId),
                Text("chill.context.session_id", record.SessionId),
                Text("chill.privacy.consent", "granted"),
                Text("chill.privacy.policy_version", configuration.PolicyVersion),
                Text("chill.privacy.capture_class", "analytics"),
                Text("chill.privacy.redaction_state", "none")
            };

            for (int index = 0; index < annotations.Count; index += 1)
            {
                ChillAnnotationEntry annotation = annotations[index];
                ChillAnnotationClassification classification;
                if (!definitions.TryGetValue(annotation.Key, out classification))
                {
                    continue;
                }
                attributes.Add(new ChillOtlpAttribute(
                    "chill.annotation." + annotation.Key,
                    ChillJson.AnyValue(annotation.Value)));
                attributes.Add(Text(
                    "chill.privacy.annotation_classification." + annotation.Key,
                    ChillNames.SnakeCase(classification)));
            }
            return attributes;
        }

        internal static string Batch(
            ChillConfiguration configuration,
            string sourcePlatform,
            string processId,
            string unityVersion,
            string operatingSystem,
            IReadOnlyList<ChillQueuedRecord> records)
        {
            var logs = new List<string>(records.Count);
            for (int index = 0; index < records.Count; index += 1)
            {
                logs.Add(records[index].LogJson);
            }
            var resource = new List<string>
            {
                ChillJson.Attribute("service.name", ChillJson.StringValue(configuration.ServiceName)),
                ChillJson.Attribute("telemetry.sdk.language", ChillJson.StringValue("csharp")),
                ChillJson.Attribute("process.runtime.name", ChillJson.StringValue("unity")),
                ChillJson.Attribute("process.runtime.version", ChillJson.StringValue(unityVersion)),
                ChillJson.Attribute("os.type", ChillJson.StringValue(operatingSystem)),
                ChillJson.Attribute("chill.game.engine", ChillJson.StringValue("unity")),
                ChillJson.Attribute("chill.source.platform", ChillJson.StringValue(sourcePlatform)),
                ChillJson.Attribute("chill.source.process_id", ChillJson.StringValue(processId))
            };

            return "{\"resourceLogs\":[{\"resource\":{\"attributes\":"
                + ChillJson.Array(resource)
                + "},\"scopeLogs\":[{\"scope\":{\"name\":\"dev.chill.unity\",\"version\":\"0.1.0\"},"
                + "\"schemaUrl\":\"" + Schema + "\",\"logRecords\":"
                + ChillJson.Array(logs)
                + "}]}]}";
        }

        internal static ChillOtlpAttribute Text(string key, string value)
        {
            return new ChillOtlpAttribute(key, ChillJson.StringValue(value));
        }

        internal static ChillOtlpAttribute Boolean(string key, bool value)
        {
            return new ChillOtlpAttribute(key, ChillJson.BooleanValue(value));
        }

        internal static ChillOtlpAttribute Integer(string key, long value)
        {
            return new ChillOtlpAttribute(key, ChillJson.IntegerValue(value));
        }

        internal static ChillOtlpAttribute Integer(string key, string value)
        {
            return new ChillOtlpAttribute(key, ChillJson.IntegerValue(value));
        }

        internal static ChillOtlpAttribute Number(string key, double value)
        {
            return new ChillOtlpAttribute(key, ChillJson.DoubleValue(value));
        }

        internal static ChillOtlpAttribute Strings(string key, IReadOnlyList<string> values)
        {
            return new ChillOtlpAttribute(key, ChillJson.StringArrayValue(values));
        }
    }
}
