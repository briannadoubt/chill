// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"
#include "ChillStore.h"
#include "ChillTypes.h"

class FChillConfiguration;

/** One encoded OTLP key/value pair. */
struct CHILLOBSERVABILITY_API FChillOtlpAttribute
{
	FChillOtlpAttribute() = default;
	FChillOtlpAttribute(FString InKey, FString InValueJson)
		: Key(MoveTemp(InKey))
		, ValueJson(MoveTemp(InValueJson))
	{
	}

	FString Key;
	FString ValueJson;

	FString ToJson() const;
};

/** One behavior record, before it is queued as an OTLP log record. */
struct CHILLOBSERVABILITY_API FChillRecord
{
	FString RecordId;
	FString SubjectId;
	FString Kind;
	FString Operation;
	FString Name;
	FString OccurredAtUnixNanoseconds;
	FString MonotonicNanoseconds;
	int64 Sequence = 0;
	FString SessionId;
	FString Traceparent;
	TArray<FChillOtlpAttribute> Attributes;

	FString ToLogJson() const;
};

/** A peeked set of queued records plus the encoded body that carries them. */
struct CHILLOBSERVABILITY_API FChillExportBatch
{
	TArray<FChillQueuedRecord> Records;
	FString Body;
};

/** OTLP envelope construction. */
namespace ChillOtlp
{
	CHILLOBSERVABILITY_API extern const TCHAR* const SchemaUrl;

	CHILLOBSERVABILITY_API TArray<FChillOtlpAttribute> BaseAttributes(
		const FChillConfiguration& Configuration,
		const FChillRecord& Record,
		const FString& SourcePlatform,
		const FString& InstallationId,
		const TArray<FChillAnnotationEntry>& Annotations,
		const TMap<FString, EChillAnnotationClassification>& Definitions);

	CHILLOBSERVABILITY_API FString Batch(
		const FChillConfiguration& Configuration,
		const FString& SourcePlatform,
		const FString& ProcessId,
		const FString& EngineVersion,
		const FString& OperatingSystem,
		const TArray<FChillQueuedRecord>& Records);

	CHILLOBSERVABILITY_API FChillOtlpAttribute Text(const FString& Key, const FString& Value);
	CHILLOBSERVABILITY_API FChillOtlpAttribute Boolean(const FString& Key, bool Value);
	CHILLOBSERVABILITY_API FChillOtlpAttribute Integer(const FString& Key, int64 Value);
	CHILLOBSERVABILITY_API FChillOtlpAttribute IntegerText(const FString& Key, const FString& Value);
	CHILLOBSERVABILITY_API FChillOtlpAttribute Number(const FString& Key, double Value);
	CHILLOBSERVABILITY_API FChillOtlpAttribute Strings(const FString& Key, const TArray<FString>& Values);
}
