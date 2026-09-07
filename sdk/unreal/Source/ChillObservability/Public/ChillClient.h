// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"
#include "ChillClock.h"
#include "ChillConfiguration.h"
#include "ChillRecord.h"
#include "ChillStore.h"
#include "ChillTypes.h"

class FChillClient;

/** Live page state used to stamp page context onto every record. */
struct CHILLOBSERVABILITY_API FChillPageState
{
	FString SurfaceId;
	FString InstanceId;
	TArray<FString> Path;
	TArray<FString> PathInstanceIds;
	FString Segment;
	EChillPageRelation Relation = EChillPageRelation::Root;
	FChillAnnotations Annotations;
};

/**
 * Scope handle for an activity. Ending is idempotent: the first terminal call wins
 * and later calls, including destruction, are ignored.
 */
class CHILLOBSERVABILITY_API FChillActivityScope
{
public:
	FChillActivityScope() = default;
	FChillActivityScope(
		FChillClient* InClient,
		FString InName,
		FString InSubjectId,
		FString InTraceparent,
		FChillAnnotations InAnnotations,
		double InStartedAt,
		EChillActivityKind InKind,
		TSharedPtr<FChillPageState> InPage);
	~FChillActivityScope();

	FChillActivityScope(const FChillActivityScope&) = delete;
	FChillActivityScope& operator=(const FChillActivityScope&) = delete;
	FChillActivityScope(FChillActivityScope&&) = default;
	FChillActivityScope& operator=(FChillActivityScope&&) = default;

	void Succeed();
	void Fail(const FString& ReasonCode = FString());
	void Cancel(const FString& ReasonCode = FString());
	void Timeout(const FString& ReasonCode = FString());

private:
	void Finish(EChillOutcome Outcome, const FString& ReasonCode);

	FChillClient* Client = nullptr;
	FString Name;
	FString SubjectId;
	FString Traceparent;
	FChillAnnotations Annotations;
	double StartedAt = 0.0;
	EChillActivityKind Kind = EChillActivityKind::Domain;
	TSharedPtr<FChillPageState> Page;
};

/** Scope handle for a page. Ends the page when destroyed. */
class CHILLOBSERVABILITY_API FChillPageScope
{
public:
	FChillPageScope() = default;
	FChillPageScope(FChillClient* InClient, TSharedPtr<FChillPageState> InPage);
	~FChillPageScope();

	FChillPageScope(const FChillPageScope&) = delete;
	FChillPageScope& operator=(const FChillPageScope&) = delete;
	FChillPageScope(FChillPageScope&&) = default;
	FChillPageScope& operator=(FChillPageScope&&) = default;

	void End();

private:
	FChillClient* Client = nullptr;
	TSharedPtr<FChillPageState> Page;
};

/**
 * The capture client.
 *
 * Capture is denied until consent is granted, and revoking consent purges the
 * durable queue. Every public entry point is guarded by a critical section so a
 * game thread and a worker thread can record concurrently.
 */
class CHILLOBSERVABILITY_API FChillClient
{
public:
	explicit FChillClient(const FChillConfiguration& InConfiguration);
	FChillClient(
		const FChillConfiguration& InConfiguration,
		TSharedPtr<IChillClock> InClock,
		TSharedPtr<IChillStore> InStore,
		const FString& InSourcePlatform,
		const FString& InEngineVersion,
		const FString& InOperatingSystem);
	~FChillClient();

	void Start();
	void Stop();

	bool IsStarted() const;
	EChillConsent GetConsent() const;
	TArray<FChillDiagnostic> GetDiagnostics() const;

	void SetConsent(EChillConsent Value);
	void DeclareAnnotation(
		const FString& Key,
		EChillAnnotationClassification Classification = EChillAnnotationClassification::Internal);

	bool Action(
		const FString& Name,
		const FChillAnnotations& Annotations = FChillAnnotations(),
		EChillActionActivation Activation = EChillActionActivation::Primary,
		EChillInput Input = EChillInput::Unknown,
		const FString& Role = TEXT("control"));

	bool Event(
		const FString& Name,
		const FChillAnnotations& Annotations = FChillAnnotations(),
		EChillEventClass EventClass = EChillEventClass::Domain,
		EChillEventSeverity Severity = EChillEventSeverity::Info);

	bool Impression(
		const FString& Name,
		const FChillAnnotations& Annotations = FChillAnnotations(),
		const FString& Role = TEXT("content"),
		double VisibilityRatio = 1.0);

	FChillActivityScope BeginActivity(
		const FString& Name,
		const FChillAnnotations& Annotations = FChillAnnotations(),
		EChillActivityKind Kind = EChillActivityKind::Domain);

	FChillPageScope StartPage(
		const FString& Segment,
		const FChillAnnotations& Annotations = FChillAnnotations(),
		EChillPageRelation Relation = EChillPageRelation::Root,
		EChillPageCause Cause = EChillPageCause::Navigate,
		bool bInheritCurrentPath = false);

	/** Returns false when there is nothing to send. */
	bool CreateExportBatch(FChillExportBatch& OutBatch);
	void Acknowledge(const FChillExportBatch& Batch);
	void ReportTransportFailure();

	const FString& GetEndpoint() const { return Configuration.GetEndpoint(); }
	const FString& GetSdkKey() const { return Configuration.GetSdkKey(); }
	float GetFlushIntervalSeconds() const { return Configuration.FlushIntervalSeconds; }

	void EndActivity(
		const FString& Name,
		const FString& SubjectId,
		const FString& Traceparent,
		const FChillAnnotations& Annotations,
		double StartedAt,
		EChillActivityKind Kind,
		TSharedPtr<FChillPageState> Page,
		EChillOutcome Outcome,
		const FString& ReasonCode);

	void EndPage(TSharedPtr<FChillPageState> Page, EChillPageCause Cause);

	static const int64 MaximumSafeSequence = 9007199254740991LL;

private:
	bool EmitLocked(
		const FString& Kind,
		const FString& Operation,
		const FString& Name,
		const FString& SubjectId,
		const FChillAnnotations& Annotations,
		const TArray<FChillOtlpAttribute>& Payload,
		const EChillOutcome* Outcome,
		const FString& Traceparent,
		TSharedPtr<FChillPageState> Page);

	void OpenSessionLocked();
	void EnsureCaptureStateLocked();
	TArray<FChillAnnotationEntry> FilterAnnotationsLocked(const FChillAnnotations& Annotations);
	TSharedPtr<FChillPageState> CurrentPageLocked() const;
	bool CanCaptureLocked();
	bool CanRecordLocked();
	void DiagnosticLocked(const FString& Code, const FString& Detail);

	static void AddPageContext(TArray<FChillOtlpAttribute>& Attributes, const TSharedPtr<FChillPageState>& Page);
	static TArray<FChillOtlpAttribute> PagePayload(
		const TSharedPtr<FChillPageState>& Page,
		const FString& Exposure,
		bool bFocused,
		EChillPageCause Cause);

	mutable FCriticalSection Gate;
	FChillConfiguration Configuration;
	TSharedPtr<IChillClock> Clock;
	TSharedPtr<IChillStore> Store;
	TSharedPtr<IChillStore> InjectedStore;
	TMap<FString, EChillAnnotationClassification> AnnotationDefinitions;
	TArray<FChillDiagnostic> Diagnostics;
	TArray<TSharedPtr<FChillPageState>> Pages;
	FString SourcePlatform;
	FString EngineVersion;
	FString OperatingSystem;
	FString InstallationId;
	FString BootId;
	FString ProcessId;
	FString SessionId;
	EChillConsent Consent = EChillConsent::Denied;
	int64 Sequence = 0;
	bool bSampled = false;
	bool bSampleDecisionMade = false;
	bool bStarted = false;
	bool bConfigurationValid = false;
};
