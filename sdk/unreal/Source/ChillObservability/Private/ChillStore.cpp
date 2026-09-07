// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillStore.h"

#include "ChillIds.h"
#include "HAL/FileManager.h"
#include "HAL/PlatformFileManager.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"

namespace
{
	const TCHAR* const QueueMagic = TEXT("CHILLQ1\n");

	FString PaddedNanoseconds(const FString& Value)
	{
		FString Padded = Value;
		while (Padded.Len() < 19)
		{
			Padded = TEXT("0") + Padded;
		}
		return Padded;
	}
}

const TCHAR* FChillFileStore::GetMagic()
{
	return QueueMagic;
}

bool FChillMemoryStore::Append(const FChillQueuedRecord& Record)
{
	Records.Add(Record);
	return true;
}

TArray<FChillQueuedRecord> FChillMemoryStore::Peek(int32 MaximumRecords)
{
	TArray<FChillQueuedRecord> Selected;
	const int32 Count = FMath::Min(MaximumRecords, Records.Num());
	for (int32 Index = 0; Index < Count; ++Index)
	{
		Selected.Add(Records[Index]);
	}
	return Selected;
}

void FChillMemoryStore::Acknowledge(const TArray<FChillQueuedRecord>& Acknowledged)
{
	const int32 Count = FMath::Min(Acknowledged.Num(), Records.Num());
	if (Count > 0)
	{
		Records.RemoveAt(0, Count);
	}
}

void FChillMemoryStore::Purge()
{
	Records.Reset();
}

FChillFileStore::FChillFileStore(
	const FString& InDirectory,
	int64 InMaximumBytes,
	TFunction<void(const FString&)> InDiagnostic)
	: Directory(InDirectory)
	, MaximumBytes(InMaximumBytes)
	, Diagnostic(MoveTemp(InDiagnostic))
{
	IFileManager::Get().MakeDirectory(*Directory, true);
	Recover();
}

bool FChillFileStore::Append(const FChillQueuedRecord& Record)
{
	const FString Payload = FString(QueueMagic) + Record.LogJson;
	FTCHARToUTF8 Converted(*Payload);
	const int64 ByteCount = Converted.Length();
	if (CurrentBytes + ByteCount > MaximumBytes)
	{
		Diagnostic(TEXT("queue_full"));
		return false;
	}

	const FString Prefix = PaddedNanoseconds(Record.OccurredAtUnixNanoseconds)
		+ TEXT("-")
		+ FString::Printf(TEXT("%016lld"), Record.Sequence)
		+ TEXT("-")
		+ Record.RecordId;
	const FString FinalPath = FPaths::Combine(Directory, Prefix + TEXT(".json"));
	const FString TemporaryPath = FPaths::Combine(Directory, TEXT(".") + Prefix + TEXT(".tmp"));

	// Write to a temporary file first, then move it into place, so a crash mid-write
	// never leaves a partially written record in the readable queue.
	if (!FFileHelper::SaveStringToFile(
			Payload,
			*TemporaryPath,
			FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM))
	{
		IFileManager::Get().Delete(*TemporaryPath, false, true, true);
		Diagnostic(TEXT("storage_failed"));
		return false;
	}
	if (!IFileManager::Get().Move(*FinalPath, *TemporaryPath, true, true))
	{
		IFileManager::Get().Delete(*TemporaryPath, false, true, true);
		Diagnostic(TEXT("storage_failed"));
		return false;
	}
	CurrentBytes += ByteCount;
	return true;
}

TArray<FChillQueuedRecord> FChillFileStore::Peek(int32 MaximumRecords)
{
	TArray<FChillQueuedRecord> Selected;
	TArray<FString> Files;
	IFileManager::Get().FindFiles(Files, *(Directory / TEXT("*.json")), true, false);
	Files.Sort();

	for (int32 Index = 0; Index < Files.Num() && Selected.Num() < MaximumRecords; ++Index)
	{
		const FString FullPath = FPaths::Combine(Directory, Files[Index]);
		FString Body;
		if (!FFileHelper::LoadFileToString(Body, *FullPath))
		{
			Quarantine(FullPath, true);
			continue;
		}
		if (!Body.StartsWith(QueueMagic, ESearchCase::CaseSensitive))
		{
			Quarantine(FullPath, true);
			continue;
		}
		const FString Name = FPaths::GetBaseFilename(Files[Index]);
		TArray<FString> Parts;
		Name.ParseIntoArray(Parts, TEXT("-"), false);
		// occurredAt-sequence-<uuid with four dashes> => at least six segments.
		if (Parts.Num() < 6)
		{
			Quarantine(FullPath, true);
			continue;
		}
		const int64 Sequence = FCString::Atoi64(*Parts[1]);
		FString RecordId = Parts[2];
		for (int32 PartIndex = 3; PartIndex < Parts.Num(); ++PartIndex)
		{
			RecordId += TEXT("-") + Parts[PartIndex];
		}
		Selected.Emplace(
			RecordId,
			Sequence,
			Parts[0],
			Body.RightChop(FCString::Strlen(QueueMagic)),
			FullPath,
			IFileManager::Get().FileSize(*FullPath));
	}
	return Selected;
}

void FChillFileStore::Acknowledge(const TArray<FChillQueuedRecord>& Records)
{
	for (const FChillQueuedRecord& Record : Records)
	{
		// Only delete inside the queue directory this store owns.
		if (Record.Path.IsEmpty() || FPaths::GetPath(Record.Path) != Directory)
		{
			continue;
		}
		if (IFileManager::Get().Delete(*Record.Path, false, true, true))
		{
			CurrentBytes = FMath::Max<int64>(0, CurrentBytes - Record.StoredBytes);
		}
		else
		{
			Diagnostic(TEXT("storage_failed"));
		}
	}
}

void FChillFileStore::Purge()
{
	TArray<FString> Files;
	IFileManager::Get().FindFiles(Files, *(Directory / TEXT("*")), true, false);
	for (const FString& File : Files)
	{
		IFileManager::Get().Delete(*FPaths::Combine(Directory, File), false, true, true);
	}
	CurrentBytes = 0;
}

void FChillFileStore::Recover()
{
	TArray<FString> Temporary;
	IFileManager::Get().FindFiles(Temporary, *(Directory / TEXT("*.tmp")), true, false);
	for (const FString& File : Temporary)
	{
		IFileManager::Get().Delete(*FPaths::Combine(Directory, File), false, true, true);
	}

	TArray<FString> Files;
	IFileManager::Get().FindFiles(Files, *(Directory / TEXT("*.json")), true, false);
	for (const FString& File : Files)
	{
		const FString FullPath = FPaths::Combine(Directory, File);
		FString Body;
		if (!FFileHelper::LoadFileToString(Body, *FullPath)
			|| !Body.StartsWith(QueueMagic, ESearchCase::CaseSensitive))
		{
			Quarantine(FullPath, false);
			continue;
		}
		CurrentBytes += IFileManager::Get().FileSize(*FullPath);
	}
}

void FChillFileStore::Quarantine(const FString& Path, bool bWasCounted)
{
	int64 StoredBytes = 0;
	if (bWasCounted)
	{
		StoredBytes = FMath::Max<int64>(0, IFileManager::Get().FileSize(*Path));
	}

	FString Target = Path + TEXT(".corrupt");
	if (IFileManager::Get().FileExists(*Target))
	{
		Target += TEXT(".") + FString::Printf(TEXT("%lld"), FDateTime::UtcNow().ToUnixTimestamp() * 1000);
	}
	if (IFileManager::Get().Move(*Target, *Path, true, true))
	{
		Diagnostic(TEXT("queue_corrupt"));
	}
	else
	{
		IFileManager::Get().Delete(*Path, false, true, true);
		Diagnostic(TEXT("storage_failed"));
	}
	CurrentBytes = FMath::Max<int64>(0, CurrentBytes - StoredBytes);
}

namespace ChillInstallation
{
	FString LoadOrCreate(const FString& QueueDirectory, const FString& Configured)
	{
		if (!Configured.IsEmpty())
		{
			return Configured;
		}

		const FString Parent = FPaths::GetPath(QueueDirectory.TrimEnd());
		const FString Directory = Parent.IsEmpty() ? QueueDirectory : Parent;
		IFileManager::Get().MakeDirectory(*Directory, true);
		const FString Path = FPaths::Combine(Directory, TEXT("installation-id"));

		FString Existing;
		if (FFileHelper::LoadFileToString(Existing, *Path))
		{
			Existing.TrimStartAndEndInline();
			if (ChillIds::IsUuidV4(Existing))
			{
				return Existing;
			}
		}

		const FString Generated = ChillIds::NewUuidV4();
		const FString Temporary = Path + TEXT(".tmp");
		if (FFileHelper::SaveStringToFile(
				Generated + TEXT("\n"),
				*Temporary,
				FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM))
		{
			if (!IFileManager::Get().Move(*Path, *Temporary, true, true))
			{
				IFileManager::Get().Delete(*Temporary, false, true, true);
			}
		}
		return Generated;
	}
}
