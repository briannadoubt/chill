// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillConfiguration.h"

#include "ChillIds.h"
#include "Misc/Paths.h"

FChillConfiguration::FChillConfiguration(
	const FString& InServiceName,
	const FString& InEndpoint,
	const FString& InSdkKey)
	: ServiceName(InServiceName)
	, Endpoint(InEndpoint)
	, SdkKey(InSdkKey)
{
	QueueDirectory = FPaths::Combine(FPaths::ProjectSavedDir(), TEXT("Chill"), TEXT("Queue"));
}

bool FChillConfiguration::IsValid(FString& OutError) const
{
	if (!ChillNames::IsSemantic(ServiceName))
	{
		OutError = TEXT("Service name must be a lowercase semantic identifier.");
		return false;
	}
	if (!ValidateEndpoint(Endpoint, OutError))
	{
		return false;
	}
	if (SdkKey.IsEmpty() || SdkKey.Len() > 512)
	{
		OutError = TEXT("An SDK key containing at most 512 characters is required.");
		return false;
	}
	return true;
}

bool FChillConfiguration::Validate(FString& OutError) const
{
	if (!IsValid(OutError))
	{
		return false;
	}
	if (PolicyVersion.IsEmpty() || PolicyVersion.Len() > 64)
	{
		OutError = TEXT("PolicyVersion must contain 1 to 64 characters.");
		return false;
	}
	if (QueueDirectory.IsEmpty())
	{
		OutError = TEXT("QueueDirectory is required.");
		return false;
	}
	if (MaximumQueueBytes < 1024 || MaximumQueueBytes > 1024LL * 1024LL * 1024LL)
	{
		OutError = TEXT("MaximumQueueBytes is out of range.");
		return false;
	}
	if (MaximumBatchRecords < 1 || MaximumBatchRecords > 1000)
	{
		OutError = TEXT("MaximumBatchRecords is out of range.");
		return false;
	}
	if (FlushIntervalSeconds < 1.0f || FlushIntervalSeconds > 3600.0f)
	{
		OutError = TEXT("FlushIntervalSeconds is out of range.");
		return false;
	}
	if (SampleRate < 0.0 || SampleRate > 1.0)
	{
		OutError = TEXT("SampleRate is out of range.");
		return false;
	}
	if (!InstallationId.IsEmpty() && !ChillIds::IsUuidV4(InstallationId))
	{
		OutError = TEXT("InstallationId must be a lowercase UUIDv4.");
		return false;
	}
	return true;
}

FChillConfiguration& FChillConfiguration::AllowAnnotation(
	const FString& Key,
	EChillAnnotationClassification Classification)
{
	if (ChillNames::IsAnnotationKey(Key))
	{
		AnnotationDefinitions.Add(Key, Classification);
	}
	return *this;
}

bool FChillConfiguration::ValidateEndpoint(const FString& Endpoint, FString& OutError)
{
	FString Scheme;
	FString Remainder;
	if (!Endpoint.Split(TEXT("://"), &Scheme, &Remainder))
	{
		OutError = TEXT("Endpoint must be an absolute URI.");
		return false;
	}
	Scheme = Scheme.ToLower();

	FString Authority = Remainder;
	FString PathAndBeyond;
	int32 SlashIndex = INDEX_NONE;
	if (Remainder.FindChar(TEXT('/'), SlashIndex))
	{
		Authority = Remainder.Mid(0, SlashIndex);
		PathAndBeyond = Remainder.Mid(SlashIndex);
	}

	// Credentials, query strings and fragments are all rejected outright: the
	// endpoint must be an exact /v1/logs URL.
	if (Authority.Contains(TEXT("@")))
	{
		OutError = TEXT("Endpoint must not contain credentials.");
		return false;
	}
	if (PathAndBeyond.Contains(TEXT("?")) || PathAndBeyond.Contains(TEXT("#")))
	{
		OutError = TEXT("Endpoint must not contain a query or fragment.");
		return false;
	}
	if (PathAndBeyond != TEXT("/v1/logs"))
	{
		OutError = TEXT("Endpoint path must be exactly /v1/logs.");
		return false;
	}

	FString Host = Authority;
	int32 ColonIndex = INDEX_NONE;
	if (Authority.FindChar(TEXT(':'), ColonIndex))
	{
		Host = Authority.Mid(0, ColonIndex);
	}
	const bool bLoopback = Host == TEXT("localhost")
		|| Host == TEXT("127.0.0.1")
		|| Host == TEXT("[::1]")
		|| Host == TEXT("::1");

	if (Scheme != TEXT("https") && !(bLoopback && Scheme == TEXT("http")))
	{
		OutError = TEXT("Endpoint must use HTTPS, except for loopback development.");
		return false;
	}
	return true;
}
