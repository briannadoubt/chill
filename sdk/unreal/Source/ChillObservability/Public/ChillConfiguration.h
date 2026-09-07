// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"
#include "ChillTypes.h"

/**
 * Immutable transport identity plus mutable capture policy.
 *
 * Construction validates the endpoint and key; Validate re-checks the mutable fields
 * before the client starts, so a misconfigured game fails at start rather than
 * silently dropping capture.
 */
class CHILLOBSERVABILITY_API FChillConfiguration
{
public:
	FChillConfiguration(const FString& InServiceName, const FString& InEndpoint, const FString& InSdkKey);

	/** True when the service name, endpoint and key are all acceptable. */
	bool IsValid(FString& OutError) const;

	/** Re-checks every mutable field. Returns false with a reason on the first failure. */
	bool Validate(FString& OutError) const;

	/** Declares an annotation key the client is permitted to emit. */
	FChillConfiguration& AllowAnnotation(
		const FString& Key,
		EChillAnnotationClassification Classification = EChillAnnotationClassification::Internal);

	const FString& GetServiceName() const { return ServiceName; }
	const FString& GetEndpoint() const { return Endpoint; }
	const FString& GetSdkKey() const { return SdkKey; }
	const TMap<FString, EChillAnnotationClassification>& GetAnnotationDefinitions() const
	{
		return AnnotationDefinitions;
	}

	EChillConsent Consent = EChillConsent::Denied;
	FString PolicyVersion = TEXT("privacy-v1");
	FString QueueDirectory;
	int64 MaximumQueueBytes = 64LL * 1024LL * 1024LL;
	int32 MaximumBatchRecords = 100;
	float FlushIntervalSeconds = 10.0f;
	double SampleRate = 1.0;
	FString InstallationId;

	static bool ValidateEndpoint(const FString& Endpoint, FString& OutError);

private:
	FString ServiceName;
	FString Endpoint;
	FString SdkKey;
	TMap<FString, EChillAnnotationClassification> AnnotationDefinitions;
};
