// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"
#include "ChillTypes.h"

/**
 * Minimal OTLP/JSON encoding.
 *
 * The envelope is written by hand rather than through a serializer so the emitted
 * bytes stay identical to the other Chill SDKs and are diffable against the
 * captured normalizer fixture.
 */
namespace ChillJson
{
	CHILLOBSERVABILITY_API FString Quote(const FString& Value);
	CHILLOBSERVABILITY_API FString StringValue(const FString& Value);
	CHILLOBSERVABILITY_API FString BooleanValue(bool Value);
	CHILLOBSERVABILITY_API FString IntegerValue(int64 Value);
	CHILLOBSERVABILITY_API FString IntegerValue(const FString& Value);
	CHILLOBSERVABILITY_API FString DoubleValue(double Value);
	CHILLOBSERVABILITY_API FString StringArrayValue(const TArray<FString>& Values);
	CHILLOBSERVABILITY_API FString Attribute(const FString& Key, const FString& AnyValueJson);
	CHILLOBSERVABILITY_API FString AnyValue(const FChillValue& Value);
	CHILLOBSERVABILITY_API FString Array(const TArray<FString>& EncodedValues);
}
