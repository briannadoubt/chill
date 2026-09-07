// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillClient.h"

#include "ChillIds.h"
#include "ChillJson.h"
#include "HAL/PlatformProperties.h"
#include "HAL/PlatformTime.h"
#include "Misc/App.h"

namespace
{
	constexpr int32 MaximumDiagnostics = 64;
	constexpr int32 MaximumAnnotations = 128;
	constexpr int32 MaximumPageDepth = 32;

	FString CurrentSourcePlatform()
	{
		// Mirrors the Unity mapping: mobile and console targets report their family,
		// everything else reports "server".
#if PLATFORM_ANDROID
		return TEXT("android");
#elif PLATFORM_IOS || PLATFORM_MAC || PLATFORM_TVOS
		return TEXT("apple");
#else
		return TEXT("server");
#endif
	}
}

FChillClient::FChillClient(const FChillConfiguration& InConfiguration)
	: FChillClient(
		InConfiguration,
		MakeShared<FChillSystemClock>(),
		nullptr,
		CurrentSourcePlatform(),
		FString(TEXT("5.4")),
		FString(FPlatformProperties::IniPlatformName()).ToLower())
{
}

FChillClient::FChillClient(
	const FChillConfiguration& InConfiguration,
	TSharedPtr<IChillClock> InClock,
	TSharedPtr<IChillStore> InStore,
	const FString& InSourcePlatform,
	const FString& InEngineVersion,
	const FString& InOperatingSystem)
	: Configuration(InConfiguration)
	, Clock(MoveTemp(InClock))
	, InjectedStore(MoveTemp(InStore))
	, SourcePlatform(InSourcePlatform)
	, EngineVersion(InEngineVersion)
	, OperatingSystem(InOperatingSystem)
{
	FString Error;
	bConfigurationValid = Configuration.Validate(Error);
	Consent = Configuration.Consent;
	AnnotationDefinitions = Configuration.GetAnnotationDefinitions();
	if (!bConfigurationValid)
	{
		Diagnostics.Emplace(TEXT("invalid_configuration"), Error);
	}
}

FChillClient::~FChillClient()
{
	Stop();
}

void FChillClient::Start()
{
	FScopeLock Lock(&Gate);
	if (bStarted || !bConfigurationValid)
	{
		return;
	}
	bStarted = true;
	if (CanCaptureLocked())
	{
		OpenSessionLocked();
	}
}

void FChillClient::Stop()
{
	FScopeLock Lock(&Gate);
	if (!bStarted)
	{
		return;
	}
	for (int32 Index = Pages.Num() - 1; Index >= 0; --Index)
	{
		TSharedPtr<FChillPageState> Page = Pages[Index];
		const TArray<FChillOtlpAttribute> Payload =
			PagePayload(Page, TEXT("retained"), false, EChillPageCause::SurfaceDestroyed);
		const EChillOutcome Outcome = EChillOutcome::Ok;
		EmitLocked(TEXT("page"), TEXT("end"), Page->Segment, Page->InstanceId,
			Page->Annotations, Payload, &Outcome, FString(), Page);
		Pages.RemoveAt(Index);
	}
	if (CanRecordLocked() && !SessionId.IsEmpty())
	{
		TArray<FChillOtlpAttribute> Payload;
		Payload.Add(ChillOtlp::Text(TEXT("chill.payload.end_reason"), TEXT("terminated")));
		const EChillOutcome Outcome = EChillOutcome::Ok;
		EmitLocked(TEXT("session"), TEXT("end"), TEXT("app.session"), SessionId,
			FChillAnnotations(), Payload, &Outcome, FString(), nullptr);
	}
	SessionId.Reset();
	bStarted = false;
}

bool FChillClient::IsStarted() const
{
	FScopeLock Lock(&Gate);
	return bStarted;
}

EChillConsent FChillClient::GetConsent() const
{
	FScopeLock Lock(&Gate);
	return Consent;
}

TArray<FChillDiagnostic> FChillClient::GetDiagnostics() const
{
	FScopeLock Lock(&Gate);
	return Diagnostics;
}

void FChillClient::SetConsent(EChillConsent Value)
{
	FScopeLock Lock(&Gate);
	if (Consent == Value)
	{
		return;
	}
	Consent = Value;
	if (Value == EChillConsent::Denied)
	{
		// Revoking consent discards everything already captured, including the
		// durable queue, so nothing recorded under consent can still be sent.
		SessionId.Reset();
		Pages.Reset();
		if (Store.IsValid())
		{
			Store->Purge();
		}
		return;
	}
	if (bStarted && CanCaptureLocked())
	{
		OpenSessionLocked();
	}
}

void FChillClient::DeclareAnnotation(const FString& Key, EChillAnnotationClassification Classification)
{
	if (!ChillNames::IsAnnotationKey(Key))
	{
		return;
	}
	FScopeLock Lock(&Gate);
	if (const EChillAnnotationClassification* Existing = AnnotationDefinitions.Find(Key))
	{
		if (*Existing != Classification)
		{
			DiagnosticLocked(TEXT("annotation_classification_conflict"), Key);
			return;
		}
	}
	AnnotationDefinitions.Add(Key, Classification);
}

bool FChillClient::Action(
	const FString& Name,
	const FChillAnnotations& Annotations,
	EChillActionActivation Activation,
	EChillInput Input,
	const FString& Role)
{
	if (!ChillNames::IsSemantic(Role))
	{
		return false;
	}
	TArray<FChillOtlpAttribute> Payload;
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.element_id"), Name));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.role"), Role));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.activation"), ChillNames::SnakeCase(Activation)));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.input"), ChillNames::SnakeCase(Input)));

	FScopeLock Lock(&Gate);
	return EmitLocked(TEXT("action"), TEXT("instant"), Name, FString(),
		Annotations, Payload, nullptr, FString(), nullptr);
}

bool FChillClient::Event(
	const FString& Name,
	const FChillAnnotations& Annotations,
	EChillEventClass EventClass,
	EChillEventSeverity Severity)
{
	TArray<FChillOtlpAttribute> Payload;
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.event_class"), ChillNames::SnakeCase(EventClass)));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.severity"), ChillNames::SnakeCase(Severity)));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.emission"), TEXT("observed")));

	FScopeLock Lock(&Gate);
	return EmitLocked(TEXT("event"), TEXT("instant"), Name, FString(),
		Annotations, Payload, nullptr, FString(), nullptr);
}

bool FChillClient::Impression(
	const FString& Name,
	const FChillAnnotations& Annotations,
	const FString& Role,
	double VisibilityRatio)
{
	if (!ChillNames::IsSemantic(Role) || VisibilityRatio < 0.0 || VisibilityRatio > 1.0)
	{
		return false;
	}
	TArray<FChillOtlpAttribute> Payload;
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.element_id"), Name));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.role"), Role));
	Payload.Add(ChillOtlp::Number(TEXT("chill.payload.visibility_ratio"), VisibilityRatio));

	FScopeLock Lock(&Gate);
	return EmitLocked(TEXT("impression"), TEXT("instant"), Name, FString(),
		Annotations, Payload, nullptr, FString(), nullptr);
}

FChillActivityScope FChillClient::BeginActivity(
	const FString& Name,
	const FChillAnnotations& Annotations,
	EChillActivityKind Kind)
{
	if (!ChillNames::IsSemantic(Name))
	{
		return FChillActivityScope();
	}
	FScopeLock Lock(&Gate);
	if (!CanRecordLocked())
	{
		return FChillActivityScope();
	}
	const FString SubjectId = ChillIds::NewUuidV7(Clock->UnixMilliseconds());
	const FString Traceparent = ChillIds::NewTraceparent();
	TSharedPtr<FChillPageState> Page = CurrentPageLocked();

	TArray<FChillOtlpAttribute> Payload;
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.activity_kind"), ChillNames::SnakeCase(Kind)));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.role"), TEXT("operation")));
	Payload.Add(ChillOtlp::Integer(TEXT("chill.payload.attempt"), 1));
	Payload.Add(ChillOtlp::Integer(TEXT("chill.payload.recursion_depth"), 0));

	const bool bEmitted = EmitLocked(TEXT("activity"), TEXT("start"), Name, SubjectId,
		Annotations, Payload, nullptr, Traceparent, Page);
	if (!bEmitted)
	{
		return FChillActivityScope();
	}
	return FChillActivityScope(
		this, Name, SubjectId, Traceparent, Annotations, FPlatformTime::Seconds(), Kind, Page);
}

FChillPageScope FChillClient::StartPage(
	const FString& Segment,
	const FChillAnnotations& Annotations,
	EChillPageRelation Relation,
	EChillPageCause Cause,
	bool bInheritCurrentPath)
{
	if (!ChillNames::IsSemantic(Segment))
	{
		return FChillPageScope();
	}
	FScopeLock Lock(&Gate);
	if (!CanRecordLocked())
	{
		return FChillPageScope();
	}

	TSharedPtr<FChillPageState> Parent = bInheritCurrentPath ? CurrentPageLocked() : nullptr;
	const FString InstanceId = ChillIds::NewUuidV7(Clock->UnixMilliseconds());

	TSharedPtr<FChillPageState> Page = MakeShared<FChillPageState>();
	if (Parent.IsValid())
	{
		Page->Path = Parent->Path;
		Page->PathInstanceIds = Parent->PathInstanceIds;
	}
	if (Page->Path.Num() >= MaximumPageDepth)
	{
		DiagnosticLocked(TEXT("page_depth_exceeded"), Segment);
		return FChillPageScope();
	}
	Page->Path.Add(Segment);
	Page->PathInstanceIds.Add(InstanceId);
	Page->SurfaceId = Parent.IsValid() ? Parent->SurfaceId : InstanceId;
	Page->InstanceId = InstanceId;
	Page->Segment = Segment;
	Page->Relation = Relation;
	Page->Annotations = Annotations;
	Pages.Add(Page);

	const TArray<FChillOtlpAttribute> Payload = PagePayload(Page, TEXT("visible"), true, Cause);
	const bool bEmitted = EmitLocked(TEXT("page"), TEXT("start"), Segment, InstanceId,
		Annotations, Payload, nullptr, FString(), Page);
	if (!bEmitted)
	{
		Pages.Remove(Page);
		return FChillPageScope();
	}
	return FChillPageScope(this, Page);
}

bool FChillClient::CreateExportBatch(FChillExportBatch& OutBatch)
{
	FScopeLock Lock(&Gate);
	if (!CanRecordLocked() || !Store.IsValid())
	{
		return false;
	}
	TArray<FChillQueuedRecord> Records = Store->Peek(Configuration.MaximumBatchRecords);
	if (Records.Num() == 0)
	{
		return false;
	}
	OutBatch.Body = ChillOtlp::Batch(
		Configuration, SourcePlatform, ProcessId, EngineVersion, OperatingSystem, Records);
	OutBatch.Records = MoveTemp(Records);
	return true;
}

void FChillClient::Acknowledge(const FChillExportBatch& Batch)
{
	FScopeLock Lock(&Gate);
	if (Consent == EChillConsent::Granted && Store.IsValid())
	{
		Store->Acknowledge(Batch.Records);
	}
}

void FChillClient::ReportTransportFailure()
{
	FScopeLock Lock(&Gate);
	DiagnosticLocked(TEXT("flush_failed"), TEXT("transport"));
}

void FChillClient::EndActivity(
	const FString& Name,
	const FString& SubjectId,
	const FString& Traceparent,
	const FChillAnnotations& Annotations,
	double StartedAt,
	EChillActivityKind Kind,
	TSharedPtr<FChillPageState> Page,
	EChillOutcome Outcome,
	const FString& ReasonCode)
{
	FScopeLock Lock(&Gate);
	if (!bStarted)
	{
		return;
	}
	const double Elapsed = FMath::Max(0.0, FPlatformTime::Seconds() - StartedAt);
	const int64 DurationNanoseconds = static_cast<int64>(Elapsed * 1000000000.0);

	TArray<FChillOtlpAttribute> Payload;
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.activity_kind"), ChillNames::SnakeCase(Kind)));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.role"), TEXT("operation")));
	Payload.Add(ChillOtlp::Integer(TEXT("chill.payload.attempt"), 1));
	Payload.Add(ChillOtlp::Integer(TEXT("chill.payload.recursion_depth"), 0));
	Payload.Add(ChillOtlp::IntegerText(
		TEXT("chill.duration_nano"), FString::Printf(TEXT("%lld"), DurationNanoseconds)));
	if (!ReasonCode.IsEmpty() && ChillNames::IsSemantic(ReasonCode))
	{
		Payload.Add(ChillOtlp::Text(TEXT("chill.outcome.reason_code"), ReasonCode));
	}
	EmitLocked(TEXT("activity"), TEXT("end"), Name, SubjectId,
		Annotations, Payload, &Outcome, Traceparent, Page);
}

void FChillClient::EndPage(TSharedPtr<FChillPageState> Page, EChillPageCause Cause)
{
	FScopeLock Lock(&Gate);
	if (!Page.IsValid() || !Pages.Contains(Page))
	{
		return;
	}
	const TArray<FChillOtlpAttribute> Payload = PagePayload(Page, TEXT("retained"), false, Cause);
	const EChillOutcome Outcome = EChillOutcome::Ok;
	EmitLocked(TEXT("page"), TEXT("end"), Page->Segment, Page->InstanceId,
		Page->Annotations, Payload, &Outcome, FString(), Page);
	Pages.Remove(Page);
}

void FChillClient::OpenSessionLocked()
{
	EnsureCaptureStateLocked();
	if (!SessionId.IsEmpty())
	{
		return;
	}
	SessionId = ChillIds::NewUuidV7(Clock->UnixMilliseconds());
	EmitLocked(TEXT("session"), TEXT("start"), TEXT("app.session"), SessionId,
		FChillAnnotations(), TArray<FChillOtlpAttribute>(), nullptr, FString(), nullptr);
}

void FChillClient::EnsureCaptureStateLocked()
{
	if (!Store.IsValid())
	{
		if (InjectedStore.IsValid())
		{
			Store = InjectedStore;
		}
		else
		{
			Store = MakeShared<FChillFileStore>(
				Configuration.QueueDirectory,
				Configuration.MaximumQueueBytes,
				[this](const FString& Code) { DiagnosticLocked(Code, TEXT("queue")); });
		}
	}
	if (InstallationId.IsEmpty())
	{
		InstallationId = InjectedStore.IsValid()
			? (Configuration.InstallationId.IsEmpty() ? ChillIds::NewUuidV4() : Configuration.InstallationId)
			: ChillInstallation::LoadOrCreate(Configuration.QueueDirectory, Configuration.InstallationId);
	}
	if (ProcessId.IsEmpty())
	{
		ProcessId = ChillIds::NewUuidV4();
	}
	if (BootId.IsEmpty())
	{
		BootId = ChillIds::NewUuidV4();
	}
}

bool FChillClient::EmitLocked(
	const FString& Kind,
	const FString& Operation,
	const FString& Name,
	const FString& SubjectId,
	const FChillAnnotations& Annotations,
	const TArray<FChillOtlpAttribute>& Payload,
	const EChillOutcome* Outcome,
	const FString& Traceparent,
	TSharedPtr<FChillPageState> Page)
{
	if (!ChillNames::IsSemantic(Name))
	{
		DiagnosticLocked(TEXT("invalid_name"), Name.IsEmpty() ? TEXT("null") : Name);
		return false;
	}
	if (!CanRecordLocked() || Sequence >= MaximumSafeSequence)
	{
		return false;
	}
	EnsureCaptureStateLocked();

	const int64 NextSequence = Sequence + 1;
	const FString RecordId = ChillIds::NewUuidV7(Clock->UnixMilliseconds());

	FChillRecord Record;
	Record.RecordId = RecordId;
	Record.SubjectId = SubjectId.IsEmpty() ? RecordId : SubjectId;
	Record.Kind = Kind;
	Record.Operation = Operation;
	Record.Name = Name;
	Record.OccurredAtUnixNanoseconds = Clock->WallUnixNanoseconds();
	Record.MonotonicNanoseconds = Clock->MonotonicNanoseconds();
	Record.Sequence = NextSequence;
	Record.SessionId = (Kind == TEXT("session")) ? (SubjectId.IsEmpty() ? SessionId : SubjectId) : SessionId;
	Record.Traceparent = Traceparent.IsEmpty() ? ChillIds::NewTraceparent() : Traceparent;

	const TArray<FChillAnnotationEntry> Kept = FilterAnnotationsLocked(Annotations);
	TArray<FChillOtlpAttribute> Attributes = ChillOtlp::BaseAttributes(
		Configuration, Record, SourcePlatform, InstallationId, Kept, AnnotationDefinitions);
	Attributes.Add(ChillOtlp::IntegerText(TEXT("chill.clock.monotonic_nano"), Record.MonotonicNanoseconds));
	Attributes.Add(ChillOtlp::Text(TEXT("chill.clock.boot_id"), BootId));
	Attributes.Add(ChillOtlp::Text(TEXT("chill.source.process_id"), ProcessId));
	AddPageContext(Attributes, Page.IsValid() ? Page : CurrentPageLocked());
	if (Outcome != nullptr)
	{
		Attributes.Add(ChillOtlp::Text(TEXT("chill.outcome.status"), ChillNames::SnakeCase(*Outcome)));
	}
	Attributes.Append(Payload);
	Record.Attributes = MoveTemp(Attributes);

	const bool bStored = Store->Append(FChillQueuedRecord(
		Record.RecordId, Record.Sequence, Record.OccurredAtUnixNanoseconds, Record.ToLogJson()));
	if (bStored)
	{
		Sequence = NextSequence;
	}
	return bStored;
}

TArray<FChillAnnotationEntry> FChillClient::FilterAnnotationsLocked(const FChillAnnotations& Annotations)
{
	TArray<FChillAnnotationEntry> Kept;
	const TArray<FChillAnnotationEntry>& Values = Annotations.GetEntries();
	for (int32 Index = 0; Index < Values.Num() && Kept.Num() < MaximumAnnotations; ++Index)
	{
		// An annotation that was never declared is dropped, not sent: the allow-list
		// is what keeps undeclared player data off the wire.
		if (!AnnotationDefinitions.Contains(Values[Index].Key))
		{
			DiagnosticLocked(TEXT("annotation_disallowed"), Values[Index].Key);
			continue;
		}
		Kept.Add(Values[Index]);
	}
	if (Values.Num() > MaximumAnnotations)
	{
		DiagnosticLocked(TEXT("annotation_limit"), TEXT("128"));
	}
	return Kept;
}

void FChillClient::AddPageContext(
	TArray<FChillOtlpAttribute>& Attributes,
	const TSharedPtr<FChillPageState>& Page)
{
	if (!Page.IsValid())
	{
		return;
	}
	Attributes.Add(ChillOtlp::Text(TEXT("chill.context.page.surface_id"), Page->SurfaceId));
	Attributes.Add(ChillOtlp::Text(TEXT("chill.context.page.instance_id"), Page->InstanceId));
	Attributes.Add(ChillOtlp::Strings(TEXT("chill.context.page.path"), Page->Path));
	Attributes.Add(ChillOtlp::Strings(TEXT("chill.context.page.path_instance_ids"), Page->PathInstanceIds));
}

TArray<FChillOtlpAttribute> FChillClient::PagePayload(
	const TSharedPtr<FChillPageState>& Page,
	const FString& Exposure,
	bool bFocused,
	EChillPageCause Cause)
{
	TArray<FChillOtlpAttribute> Payload;
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.surface_id"), Page->SurfaceId));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.instance_id"), Page->InstanceId));
	Payload.Add(ChillOtlp::Strings(TEXT("chill.payload.path"), Page->Path));
	Payload.Add(ChillOtlp::Strings(TEXT("chill.payload.path_instance_ids"), Page->PathInstanceIds));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.relation"), ChillNames::SnakeCase(Page->Relation)));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.exposure"), Exposure));
	Payload.Add(ChillOtlp::Boolean(TEXT("chill.payload.focused"), bFocused));
	Payload.Add(ChillOtlp::Text(TEXT("chill.payload.cause"), ChillNames::SnakeCase(Cause)));
	return Payload;
}

TSharedPtr<FChillPageState> FChillClient::CurrentPageLocked() const
{
	return Pages.Num() == 0 ? nullptr : Pages.Last();
}

bool FChillClient::CanCaptureLocked()
{
	if (Consent != EChillConsent::Granted)
	{
		return false;
	}
	if (!bSampleDecisionMade)
	{
		bSampled = Configuration.SampleRate >= 1.0
			|| (Configuration.SampleRate > 0.0 && ChillIds::RandomUnit() < Configuration.SampleRate);
		bSampleDecisionMade = true;
	}
	return bSampled;
}

bool FChillClient::CanRecordLocked()
{
	return bStarted && CanCaptureLocked() && !SessionId.IsEmpty();
}

void FChillClient::DiagnosticLocked(const FString& Code, const FString& Detail)
{
	if (Diagnostics.Num() >= MaximumDiagnostics)
	{
		Diagnostics.RemoveAt(0);
	}
	Diagnostics.Emplace(Code, Detail);
}

FChillActivityScope::FChillActivityScope(
	FChillClient* InClient,
	FString InName,
	FString InSubjectId,
	FString InTraceparent,
	FChillAnnotations InAnnotations,
	double InStartedAt,
	EChillActivityKind InKind,
	TSharedPtr<FChillPageState> InPage)
	: Client(InClient)
	, Name(MoveTemp(InName))
	, SubjectId(MoveTemp(InSubjectId))
	, Traceparent(MoveTemp(InTraceparent))
	, Annotations(MoveTemp(InAnnotations))
	, StartedAt(InStartedAt)
	, Kind(InKind)
	, Page(MoveTemp(InPage))
{
}

FChillActivityScope::~FChillActivityScope()
{
	Finish(EChillOutcome::Cancelled, TEXT("scope_disposed"));
}

void FChillActivityScope::Succeed()
{
	Finish(EChillOutcome::Ok, FString());
}

void FChillActivityScope::Fail(const FString& ReasonCode)
{
	Finish(EChillOutcome::Error, ReasonCode);
}

void FChillActivityScope::Cancel(const FString& ReasonCode)
{
	Finish(EChillOutcome::Cancelled, ReasonCode);
}

void FChillActivityScope::Timeout(const FString& ReasonCode)
{
	Finish(EChillOutcome::Timeout, ReasonCode);
}

void FChillActivityScope::Finish(EChillOutcome Outcome, const FString& ReasonCode)
{
	FChillClient* Active = Client;
	if (Active == nullptr)
	{
		return;
	}
	// Clearing first makes the first terminal call win and every later call a no-op.
	Client = nullptr;
	Active->EndActivity(Name, SubjectId, Traceparent, Annotations, StartedAt, Kind, Page, Outcome, ReasonCode);
}

FChillPageScope::FChillPageScope(FChillClient* InClient, TSharedPtr<FChillPageState> InPage)
	: Client(InClient)
	, Page(MoveTemp(InPage))
{
}

FChillPageScope::~FChillPageScope()
{
	End();
}

void FChillPageScope::End()
{
	FChillClient* Active = Client;
	if (Active == nullptr)
	{
		return;
	}
	Client = nullptr;
	Active->EndPage(Page, EChillPageCause::Dismiss);
}
