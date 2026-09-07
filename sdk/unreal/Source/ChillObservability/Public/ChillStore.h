// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"

/** One record as it sits in the durable queue. */
struct CHILLOBSERVABILITY_API FChillQueuedRecord
{
	FChillQueuedRecord() = default;
	FChillQueuedRecord(
		FString InRecordId,
		int64 InSequence,
		FString InOccurredAtUnixNanoseconds,
		FString InLogJson,
		FString InPath = FString(),
		int64 InStoredBytes = 0)
		: RecordId(MoveTemp(InRecordId))
		, Sequence(InSequence)
		, OccurredAtUnixNanoseconds(MoveTemp(InOccurredAtUnixNanoseconds))
		, LogJson(MoveTemp(InLogJson))
		, Path(MoveTemp(InPath))
		, StoredBytes(InStoredBytes)
	{
	}

	FString RecordId;
	int64 Sequence = 0;
	FString OccurredAtUnixNanoseconds;
	FString LogJson;
	FString Path;
	int64 StoredBytes = 0;
};

/**
 * The durable queue boundary.
 *
 * Implementations never see the SDK key or any credential: the transport attaches
 * authorization at send time, so a leaked queue file cannot carry one.
 */
class CHILLOBSERVABILITY_API IChillStore
{
public:
	virtual ~IChillStore() = default;
	virtual bool Append(const FChillQueuedRecord& Record) = 0;
	virtual TArray<FChillQueuedRecord> Peek(int32 MaximumRecords) = 0;
	virtual void Acknowledge(const TArray<FChillQueuedRecord>& Records) = 0;
	virtual void Purge() = 0;
};

/** Fallback used when the durable queue cannot be created. */
class CHILLOBSERVABILITY_API FChillMemoryStore final : public IChillStore
{
public:
	virtual bool Append(const FChillQueuedRecord& Record) override;
	virtual TArray<FChillQueuedRecord> Peek(int32 MaximumRecords) override;
	virtual void Acknowledge(const TArray<FChillQueuedRecord>& Records) override;
	virtual void Purge() override;

private:
	TArray<FChillQueuedRecord> Records;
};

/**
 * Crash-safe file queue. Records are written to a temporary file and atomically
 * moved into place, so a torn write is never observed as a valid record.
 */
class CHILLOBSERVABILITY_API FChillFileStore final : public IChillStore
{
public:
	FChillFileStore(const FString& InDirectory, int64 InMaximumBytes, TFunction<void(const FString&)> InDiagnostic);

	virtual bool Append(const FChillQueuedRecord& Record) override;
	virtual TArray<FChillQueuedRecord> Peek(int32 MaximumRecords) override;
	virtual void Acknowledge(const TArray<FChillQueuedRecord>& Records) override;
	virtual void Purge() override;

	static const TCHAR* GetMagic();

private:
	void Recover();
	void Quarantine(const FString& Path, bool bWasCounted);

	FString Directory;
	int64 MaximumBytes = 0;
	int64 CurrentBytes = 0;
	TFunction<void(const FString&)> Diagnostic;
};

/** Loads or creates the durable installation identifier next to the queue. */
namespace ChillInstallation
{
	CHILLOBSERVABILITY_API FString LoadOrCreate(const FString& QueueDirectory, const FString& Configured);
}
