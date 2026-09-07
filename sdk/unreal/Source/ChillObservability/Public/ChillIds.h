// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"

/** Identifier generation shared by records, sessions, pages and traces. */
namespace ChillIds
{
	CHILLOBSERVABILITY_API bool IsUuidV4(const FString& Value);
	CHILLOBSERVABILITY_API FString NewUuidV4();

	/** Time-ordered UUIDv7 so queued records sort by occurrence on disk. */
	CHILLOBSERVABILITY_API FString NewUuidV7(int64 UnixMilliseconds);

	CHILLOBSERVABILITY_API FString NewTraceparent();
	CHILLOBSERVABILITY_API double RandomUnit();
	CHILLOBSERVABILITY_API FString TraceId(const FString& Traceparent);
	CHILLOBSERVABILITY_API FString SpanId(const FString& Traceparent);
}
