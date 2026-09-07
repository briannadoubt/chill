// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillRecord.h"

#include "ChillConfiguration.h"
#include "ChillIds.h"
#include "ChillJson.h"

FString FChillOtlpAttribute::ToJson() const
{
	return ChillJson::Attribute(Key, ValueJson);
}

FString FChillRecord::ToLogJson() const
{
	TArray<FString> EncodedAttributes;
	EncodedAttributes.Reserve(Attributes.Num());
	for (const FChillOtlpAttribute& Attribute : Attributes)
	{
		EncodedAttributes.Add(Attribute.ToJson());
	}

	FString Result = TEXT("{\"timeUnixNano\":");
	Result.Append(ChillJson::Quote(OccurredAtUnixNanoseconds));
	Result.Append(TEXT(",\"observedTimeUnixNano\":"));
	Result.Append(ChillJson::Quote(OccurredAtUnixNanoseconds));
	Result.Append(TEXT(",\"eventName\":"));
	Result.Append(ChillJson::Quote(Name));
	if (!Traceparent.IsEmpty())
	{
		Result.Append(TEXT(",\"traceId\":"));
		Result.Append(ChillJson::Quote(ChillIds::TraceId(Traceparent)));
		Result.Append(TEXT(",\"spanId\":"));
		Result.Append(ChillJson::Quote(ChillIds::SpanId(Traceparent)));
		Result.Append(TEXT(",\"flags\":1"));
	}
	Result.Append(TEXT(",\"attributes\":"));
	Result.Append(ChillJson::Array(EncodedAttributes));
	Result.AppendChar(TEXT('}'));
	return Result;
}

namespace ChillOtlp
{
	const TCHAR* const SchemaUrl = TEXT("https://schemas.chill.dev/behavior/v1/envelope.schema.json");

	TArray<FChillOtlpAttribute> BaseAttributes(
		const FChillConfiguration& Configuration,
		const FChillRecord& Record,
		const FString& SourcePlatform,
		const FString& InstallationId,
		const TArray<FChillAnnotationEntry>& Annotations,
		const TMap<FString, EChillAnnotationClassification>& Definitions)
	{
		TArray<FChillOtlpAttribute> Attributes;
		Attributes.Add(Text(TEXT("chill.schema.version"), TEXT("1.0.0")));
		Attributes.Add(Text(TEXT("chill.schema.url"), SchemaUrl));
		Attributes.Add(Text(TEXT("chill.record.id"), Record.RecordId));
		Attributes.Add(Text(TEXT("chill.subject.id"), Record.SubjectId));
		Attributes.Add(Text(TEXT("chill.behavior.kind"), Record.Kind));
		Attributes.Add(Text(TEXT("chill.behavior.operation"), Record.Operation));
		Attributes.Add(Text(TEXT("chill.behavior.name"), Record.Name));
		Attributes.Add(Integer(TEXT("chill.clock.sequence_number"), Record.Sequence));
		Attributes.Add(Text(TEXT("chill.source.platform"), SourcePlatform));
		Attributes.Add(Text(TEXT("chill.source.installation_id"), InstallationId));
		Attributes.Add(Text(TEXT("chill.context.session_id"), Record.SessionId));
		Attributes.Add(Text(TEXT("chill.privacy.consent"), TEXT("granted")));
		Attributes.Add(Text(TEXT("chill.privacy.policy_version"), Configuration.PolicyVersion));
		Attributes.Add(Text(TEXT("chill.privacy.capture_class"), TEXT("analytics")));
		Attributes.Add(Text(TEXT("chill.privacy.redaction_state"), TEXT("none")));

		for (const FChillAnnotationEntry& Annotation : Annotations)
		{
			const EChillAnnotationClassification* Classification = Definitions.Find(Annotation.Key);
			if (Classification == nullptr)
			{
				continue;
			}
			Attributes.Emplace(
				TEXT("chill.annotation.") + Annotation.Key,
				ChillJson::AnyValue(Annotation.Value));
			Attributes.Add(Text(
				TEXT("chill.privacy.annotation_classification.") + Annotation.Key,
				ChillNames::SnakeCase(*Classification)));
		}
		return Attributes;
	}

	FString Batch(
		const FChillConfiguration& Configuration,
		const FString& SourcePlatform,
		const FString& ProcessId,
		const FString& EngineVersion,
		const FString& OperatingSystem,
		const TArray<FChillQueuedRecord>& Records)
	{
		TArray<FString> Logs;
		Logs.Reserve(Records.Num());
		for (const FChillQueuedRecord& Record : Records)
		{
			Logs.Add(Record.LogJson);
		}

		TArray<FString> Resource;
		Resource.Add(ChillJson::Attribute(TEXT("service.name"), ChillJson::StringValue(Configuration.GetServiceName())));
		Resource.Add(ChillJson::Attribute(TEXT("telemetry.sdk.language"), ChillJson::StringValue(TEXT("cpp"))));
		Resource.Add(ChillJson::Attribute(TEXT("process.runtime.name"), ChillJson::StringValue(TEXT("unreal"))));
		Resource.Add(ChillJson::Attribute(TEXT("process.runtime.version"), ChillJson::StringValue(EngineVersion)));
		Resource.Add(ChillJson::Attribute(TEXT("os.type"), ChillJson::StringValue(OperatingSystem)));
		Resource.Add(ChillJson::Attribute(TEXT("chill.game.engine"), ChillJson::StringValue(TEXT("unreal"))));
		Resource.Add(ChillJson::Attribute(TEXT("chill.source.platform"), ChillJson::StringValue(SourcePlatform)));
		Resource.Add(ChillJson::Attribute(TEXT("chill.source.process_id"), ChillJson::StringValue(ProcessId)));

		return FString(TEXT("{\"resourceLogs\":[{\"resource\":{\"attributes\":"))
			+ ChillJson::Array(Resource)
			+ TEXT("},\"scopeLogs\":[{\"scope\":{\"name\":\"dev.chill.unreal\",\"version\":\"0.1.0\"},")
			+ TEXT("\"schemaUrl\":\"") + SchemaUrl + TEXT("\",\"logRecords\":")
			+ ChillJson::Array(Logs)
			+ TEXT("}]}]}");
	}

	FChillOtlpAttribute Text(const FString& Key, const FString& Value)
	{
		return FChillOtlpAttribute(Key, ChillJson::StringValue(Value));
	}

	FChillOtlpAttribute Boolean(const FString& Key, bool Value)
	{
		return FChillOtlpAttribute(Key, ChillJson::BooleanValue(Value));
	}

	FChillOtlpAttribute Integer(const FString& Key, int64 Value)
	{
		return FChillOtlpAttribute(Key, ChillJson::IntegerValue(Value));
	}

	FChillOtlpAttribute IntegerText(const FString& Key, const FString& Value)
	{
		return FChillOtlpAttribute(Key, ChillJson::IntegerValue(Value));
	}

	FChillOtlpAttribute Number(const FString& Key, double Value)
	{
		return FChillOtlpAttribute(Key, ChillJson::DoubleValue(Value));
	}

	FChillOtlpAttribute Strings(const FString& Key, const TArray<FString>& Values)
	{
		return FChillOtlpAttribute(Key, ChillJson::StringArrayValue(Values));
	}
}
