// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillClient.h"
#include "ChillConfiguration.h"
#include "ChillIds.h"
#include "ChillJson.h"
#include "ChillStore.h"
#include "ChillTypes.h"
#include "Misc/AutomationTest.h"

#if WITH_DEV_AUTOMATION_TESTS

namespace ChillTestSupport
{
	/** A clock pinned to fixed values so emitted records are byte-comparable. */
	class FFixedClock final : public IChillClock
	{
	public:
		virtual int64 UnixMilliseconds() const override { return 1750000000000LL; }
		virtual FString WallUnixNanoseconds() const override { return TEXT("1750000000000000000"); }
		virtual FString MonotonicNanoseconds() const override { return TEXT("42000000000"); }
	};

	/** An in-memory store that also records whether it was ever purged. */
	class FRecordingStore final : public IChillStore
	{
	public:
		virtual bool Append(const FChillQueuedRecord& Record) override
		{
			Records.Add(Record);
			return true;
		}

		virtual TArray<FChillQueuedRecord> Peek(int32 MaximumRecords) override
		{
			TArray<FChillQueuedRecord> Selected;
			const int32 Count = FMath::Min(MaximumRecords, Records.Num());
			for (int32 Index = 0; Index < Count; ++Index)
			{
				Selected.Add(Records[Index]);
			}
			return Selected;
		}

		virtual void Acknowledge(const TArray<FChillQueuedRecord>& Acknowledged) override
		{
			const int32 Count = FMath::Min(Acknowledged.Num(), Records.Num());
			if (Count > 0)
			{
				Records.RemoveAt(0, Count);
			}
		}

		virtual void Purge() override
		{
			Records.Reset();
			bPurged = true;
		}

		TArray<FChillQueuedRecord> Records;
		bool bPurged = false;
	};

	FChillConfiguration MakeConfiguration()
	{
		FChillConfiguration Configuration(
			TEXT("demo.game"), TEXT("https://collector.example.com/v1/logs"), TEXT("sdk-key"));
		Configuration.Consent = EChillConsent::Granted;
		Configuration.QueueDirectory = TEXT("/tmp/chill-unreal-tests");
		return Configuration;
	}

	TSharedPtr<FRecordingStore> MakeClient(
		TUniquePtr<FChillClient>& OutClient,
		FChillConfiguration Configuration)
	{
		TSharedPtr<FRecordingStore> Store = MakeShared<FRecordingStore>();
		OutClient = MakeUnique<FChillClient>(
			Configuration,
			MakeShared<FFixedClock>(),
			Store,
			TEXT("server"),
			TEXT("5.4"),
			TEXT("mac"));
		return Store;
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillSemanticNameTest,
	"Chill.Unreal.Names.AcceptsOnlySemanticIdentifiers",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillSemanticNameTest::RunTest(const FString&)
{
	TestTrue(TEXT("simple"), ChillNames::IsSemantic(TEXT("menu")));
	TestTrue(TEXT("dotted"), ChillNames::IsSemantic(TEXT("app.session")));
	TestTrue(TEXT("dashed"), ChillNames::IsSemantic(TEXT("main-menu")));
	TestFalse(TEXT("uppercase"), ChillNames::IsSemantic(TEXT("Menu")));
	TestFalse(TEXT("leading digit"), ChillNames::IsSemantic(TEXT("1menu")));
	TestFalse(TEXT("trailing separator"), ChillNames::IsSemantic(TEXT("menu.")));
	TestFalse(TEXT("repeated separator"), ChillNames::IsSemantic(TEXT("menu..item")));
	TestFalse(TEXT("empty"), ChillNames::IsSemantic(TEXT("")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillAnnotationKeyTest,
	"Chill.Unreal.Names.AcceptsOnlyDottedAnnotationKeys",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillAnnotationKeyTest::RunTest(const FString&)
{
	TestTrue(TEXT("simple"), ChillNames::IsAnnotationKey(TEXT("level")));
	TestTrue(TEXT("underscore"), ChillNames::IsAnnotationKey(TEXT("level_index")));
	TestTrue(TEXT("dotted"), ChillNames::IsAnnotationKey(TEXT("game.level_index")));
	TestFalse(TEXT("dash"), ChillNames::IsAnnotationKey(TEXT("game-level")));
	TestFalse(TEXT("trailing dot"), ChillNames::IsAnnotationKey(TEXT("game.")));
	TestFalse(TEXT("uppercase"), ChillNames::IsAnnotationKey(TEXT("Game")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillEndpointTest,
	"Chill.Unreal.Configuration.RejectsUnsafeEndpoints",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillEndpointTest::RunTest(const FString&)
{
	FString Error;
	TestTrue(TEXT("https"),
		FChillConfiguration::ValidateEndpoint(TEXT("https://example.com/v1/logs"), Error));
	TestTrue(TEXT("loopback http"),
		FChillConfiguration::ValidateEndpoint(TEXT("http://localhost:4318/v1/logs"), Error));
	TestFalse(TEXT("plaintext remote"),
		FChillConfiguration::ValidateEndpoint(TEXT("http://example.com/v1/logs"), Error));
	TestFalse(TEXT("wrong path"),
		FChillConfiguration::ValidateEndpoint(TEXT("https://example.com/v1/traces"), Error));
	TestFalse(TEXT("query"),
		FChillConfiguration::ValidateEndpoint(TEXT("https://example.com/v1/logs?a=b"), Error));
	TestFalse(TEXT("credentials"),
		FChillConfiguration::ValidateEndpoint(TEXT("https://user:pass@example.com/v1/logs"), Error));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillConsentGateTest,
	"Chill.Unreal.Consent.CapturesNothingUntilGranted",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillConsentGateTest::RunTest(const FString&)
{
	FChillConfiguration Configuration = ChillTestSupport::MakeConfiguration();
	Configuration.Consent = EChillConsent::Denied;
	TUniquePtr<FChillClient> Client;
	TSharedPtr<ChillTestSupport::FRecordingStore> Store =
		ChillTestSupport::MakeClient(Client, Configuration);
	Client->Start();
	TestFalse(TEXT("event refused"), Client->Event(TEXT("denied.event")));
	TestEqual(TEXT("nothing queued"), Store->Records.Num(), 0);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillConsentRevocationTest,
	"Chill.Unreal.Consent.RevocationPurgesTheQueue",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillConsentRevocationTest::RunTest(const FString&)
{
	TUniquePtr<FChillClient> Client;
	TSharedPtr<ChillTestSupport::FRecordingStore> Store =
		ChillTestSupport::MakeClient(Client, ChillTestSupport::MakeConfiguration());
	Client->Start();
	Client->Event(TEXT("domain.event"));
	TestTrue(TEXT("queued before revocation"), Store->Records.Num() > 0);
	Client->SetConsent(EChillConsent::Denied);
	TestTrue(TEXT("store purged"), Store->bPurged);
	TestEqual(TEXT("queue emptied"), Store->Records.Num(), 0);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillSessionLifecycleTest,
	"Chill.Unreal.Session.EmitsStartAndEnd",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillSessionLifecycleTest::RunTest(const FString&)
{
	TUniquePtr<FChillClient> Client;
	TSharedPtr<ChillTestSupport::FRecordingStore> Store =
		ChillTestSupport::MakeClient(Client, ChillTestSupport::MakeConfiguration());
	Client->Start();
	TestTrue(TEXT("session start queued"),
		Store->Records.Num() == 1
			&& Store->Records[0].LogJson.Contains(TEXT("\"stringValue\":\"app.session\"")));
	Client->Stop();
	TestTrue(TEXT("session end queued"),
		Store->Records.Last().LogJson.Contains(TEXT("\"stringValue\":\"terminated\"")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillUndeclaredAnnotationTest,
	"Chill.Unreal.Annotations.DropsUndeclaredKeys",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillUndeclaredAnnotationTest::RunTest(const FString&)
{
	FChillConfiguration Configuration = ChillTestSupport::MakeConfiguration();
	Configuration.AllowAnnotation(TEXT("game.mode"), EChillAnnotationClassification::Public);
	TUniquePtr<FChillClient> Client;
	TSharedPtr<ChillTestSupport::FRecordingStore> Store =
		ChillTestSupport::MakeClient(Client, Configuration);
	Client->Start();

	const FChillAnnotations Annotations = FChillAnnotations()
		.With(TEXT("game.mode"), FString(TEXT("ranked")))
		.With(TEXT("player.email"), FString(TEXT("someone@example.com")));
	Client->Event(TEXT("match.started"), Annotations);

	const FString& Json = Store->Records.Last().LogJson;
	TestTrue(TEXT("declared key kept"), Json.Contains(TEXT("chill.annotation.game.mode")));
	TestFalse(TEXT("undeclared key dropped"), Json.Contains(TEXT("player.email")));
	TestFalse(TEXT("undeclared value dropped"), Json.Contains(TEXT("someone@example.com")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillAnnotationPrecedenceTest,
	"Chill.Unreal.Annotations.AncestorWinsOnCollision",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillAnnotationPrecedenceTest::RunTest(const FString&)
{
	const FChillAnnotations Ancestor = FChillAnnotations().With(TEXT("game.mode"), FString(TEXT("ranked")));
	const FChillAnnotations Descendant = FChillAnnotations().With(TEXT("game.mode"), FString(TEXT("casual")));
	const FChillAnnotations Merged = Ancestor.MergeDescendant(Descendant);
	TestEqual(TEXT("single entry"), Merged.GetEntries().Num(), 1);
	TestEqual(TEXT("ancestor value retained"), Merged.GetEntries()[0].Value.StringValue, FString(TEXT("ranked")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillPageContextTest,
	"Chill.Unreal.Pages.StampPathContextOntoRecords",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillPageContextTest::RunTest(const FString&)
{
	TUniquePtr<FChillClient> Client;
	TSharedPtr<ChillTestSupport::FRecordingStore> Store =
		ChillTestSupport::MakeClient(Client, ChillTestSupport::MakeConfiguration());
	Client->Start();
	{
		FChillPageScope Page = Client->StartPage(TEXT("main-menu"));
		Client->Action(TEXT("play"));
		const FString& Json = Store->Records.Last().LogJson;
		TestTrue(TEXT("page path stamped"), Json.Contains(TEXT("chill.context.page.path")));
		TestTrue(TEXT("segment present"), Json.Contains(TEXT("\"stringValue\":\"main-menu\"")));
	}
	TestTrue(TEXT("page end emitted"),
		Store->Records.Last().LogJson.Contains(TEXT("\"stringValue\":\"dismiss\"")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillActivityOutcomeTest,
	"Chill.Unreal.Activities.RecordOutcomeAndDuration",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillActivityOutcomeTest::RunTest(const FString&)
{
	TUniquePtr<FChillClient> Client;
	TSharedPtr<ChillTestSupport::FRecordingStore> Store =
		ChillTestSupport::MakeClient(Client, ChillTestSupport::MakeConfiguration());
	Client->Start();
	{
		FChillActivityScope Activity = Client->BeginActivity(TEXT("level.load"), FChillAnnotations(), EChillActivityKind::Task);
		Activity.Succeed();
	}
	const FString& Json = Store->Records.Last().LogJson;
	TestTrue(TEXT("outcome recorded"), Json.Contains(TEXT("chill.outcome.status")));
	TestTrue(TEXT("ok status"), Json.Contains(TEXT("\"stringValue\":\"ok\"")));
	TestTrue(TEXT("duration recorded"), Json.Contains(TEXT("chill.duration_nano")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillActivityIdempotenceTest,
	"Chill.Unreal.Activities.EndOnlyOnce",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillActivityIdempotenceTest::RunTest(const FString&)
{
	TUniquePtr<FChillClient> Client;
	TSharedPtr<ChillTestSupport::FRecordingStore> Store =
		ChillTestSupport::MakeClient(Client, ChillTestSupport::MakeConfiguration());
	Client->Start();
	int32 CountAfterStart = 0;
	{
		FChillActivityScope Activity = Client->BeginActivity(TEXT("level.load"));
		CountAfterStart = Store->Records.Num();
		Activity.Succeed();
		Activity.Fail(TEXT("ignored"));
	}
	// Exactly one activity end record, despite the extra terminal call and destruction.
	TestEqual(TEXT("one end record"), Store->Records.Num(), CountAfterStart + 1);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillEnvelopeShapeTest,
	"Chill.Unreal.Otlp.EmitsUnrealResourceEnvelope",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillEnvelopeShapeTest::RunTest(const FString&)
{
	TUniquePtr<FChillClient> Client;
	ChillTestSupport::MakeClient(Client, ChillTestSupport::MakeConfiguration());
	Client->Start();
	Client->Event(TEXT("domain.event"));

	FChillExportBatch Batch;
	TestTrue(TEXT("batch created"), Client->CreateExportBatch(Batch));
	TestTrue(TEXT("resource logs"), Batch.Body.StartsWith(TEXT("{\"resourceLogs\":[")));
	TestTrue(TEXT("engine attribute"), Batch.Body.Contains(TEXT("\"chill.game.engine\"")));
	TestTrue(TEXT("engine is unreal"), Batch.Body.Contains(TEXT("\"stringValue\":\"unreal\"")));
	TestTrue(TEXT("cpp language"), Batch.Body.Contains(TEXT("\"stringValue\":\"cpp\"")));
	TestTrue(TEXT("scope name"), Batch.Body.Contains(TEXT("dev.chill.unreal")));
	TestFalse(TEXT("no credential on the wire body"), Batch.Body.Contains(TEXT("sdk-key")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillJsonEscapingTest,
	"Chill.Unreal.Json.EscapesControlCharacters",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillJsonEscapingTest::RunTest(const FString&)
{
	TestEqual(TEXT("quote"), ChillJson::Quote(TEXT("a\"b")), FString(TEXT("\"a\\\"b\"")));
	TestEqual(TEXT("backslash"), ChillJson::Quote(TEXT("a\\b")), FString(TEXT("\"a\\\\b\"")));
	TestEqual(TEXT("newline"), ChillJson::Quote(TEXT("a\nb")), FString(TEXT("\"a\\nb\"")));
	TestEqual(TEXT("unit separator"), ChillJson::Quote(FString::Chr(0x1f)), FString(TEXT("\"\\u001f\"")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(
	FChillIdentifierTest,
	"Chill.Unreal.Ids.GenerateVersionedUuids",
	EAutomationTestFlags_ApplicationContextMask | EAutomationTestFlags::EngineFilter)
bool FChillIdentifierTest::RunTest(const FString&)
{
	const FString Uuid4 = ChillIds::NewUuidV4();
	TestTrue(TEXT("uuidv4 shape"), ChillIds::IsUuidV4(Uuid4));

	const FString Uuid7 = ChillIds::NewUuidV7(1750000000000LL);
	TestEqual(TEXT("uuidv7 length"), Uuid7.Len(), 36);
	TestEqual(TEXT("uuidv7 version nibble"), Uuid7[14], TEXT('7'));

	const FString Traceparent = ChillIds::NewTraceparent();
	TestEqual(TEXT("traceparent length"), Traceparent.Len(), 55);
	TestEqual(TEXT("trace id length"), ChillIds::TraceId(Traceparent).Len(), 32);
	TestEqual(TEXT("span id length"), ChillIds::SpanId(Traceparent).Len(), 16);
	return true;
}

#endif // WITH_DEV_AUTOMATION_TESTS
