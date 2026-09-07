// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillSubsystem.h"

#include "HttpModule.h"
#include "Interfaces/IHttpRequest.h"
#include "Interfaces/IHttpResponse.h"

void UChillSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	TickHandle = FTSTicker::GetCoreTicker().AddTicker(
		FTickerDelegate::CreateUObject(this, &UChillSubsystem::Tick), 0.0f);
}

void UChillSubsystem::Deinitialize()
{
	Shutdown();
	if (TickHandle.IsValid())
	{
		FTSTicker::GetCoreTicker().RemoveTicker(TickHandle);
		TickHandle.Reset();
	}
	Super::Deinitialize();
}

bool UChillSubsystem::Configure(const FChillConfiguration& Configuration)
{
	if (Client.IsValid())
	{
		return false;
	}
	FString Error;
	if (!Configuration.Validate(Error))
	{
		return false;
	}
	Client = MakeUnique<FChillClient>(Configuration);
	Client->Start();
	NextFlushSeconds = FPlatformTime::Seconds() + Client->GetFlushIntervalSeconds();
	return true;
}

void UChillSubsystem::SetConsent(EChillConsent Consent)
{
	if (!Client.IsValid())
	{
		return;
	}
	if (Consent == EChillConsent::Denied && CurrentRequest.IsValid())
	{
		// Cancel any in-flight upload so nothing captured under the previous consent
		// can still reach the collector.
		CurrentRequest->CancelRequest();
		CurrentRequest.Reset();
		bFlushing = false;
	}
	Client->SetConsent(Consent);
}

void UChillSubsystem::Flush()
{
	if (bFlushing || !Client.IsValid() || Client->GetConsent() != EChillConsent::Granted)
	{
		return;
	}
	FChillExportBatch Batch;
	if (!Client->CreateExportBatch(Batch))
	{
		return;
	}
	SendBatch(Batch);
}

void UChillSubsystem::SendBatch(const FChillExportBatch& Batch)
{
	bFlushing = true;
	TSharedRef<IHttpRequest, ESPMode::ThreadSafe> Request = FHttpModule::Get().CreateRequest();
	Request->SetURL(Client->GetEndpoint());
	Request->SetVerb(TEXT("POST"));
	Request->SetHeader(TEXT("Authorization"), TEXT("Bearer ") + Client->GetSdkKey());
	Request->SetHeader(TEXT("Content-Type"), TEXT("application/json"));
	Request->SetHeader(TEXT("X-Chill-Schema-Version"), TEXT("1.0.0"));
	Request->SetContentAsString(Batch.Body);

	const TArray<FChillQueuedRecord> Records = Batch.Records;
	Request->OnProcessRequestComplete().BindLambda(
		[this, Records](FHttpRequestPtr, FHttpResponsePtr Response, bool bConnectedSuccessfully)
		{
			const int32 ResponseCode = Response.IsValid() ? Response->GetResponseCode() : 0;
			const bool bSuccess = bConnectedSuccessfully && ResponseCode >= 200 && ResponseCode < 300;
			if (Client.IsValid())
			{
				if (bSuccess)
				{
					FChillExportBatch Acknowledged;
					Acknowledged.Records = Records;
					// Records leave the queue only after the collector accepts them,
					// so a failed flush is retried rather than dropped.
					Client->Acknowledge(Acknowledged);
				}
				else
				{
					Client->ReportTransportFailure();
				}
			}
			CurrentRequest.Reset();
			bFlushing = false;
		});

	CurrentRequest = Request;
	Request->ProcessRequest();
}

bool UChillSubsystem::Tick(float DeltaSeconds)
{
	if (Client.IsValid() && FPlatformTime::Seconds() >= NextFlushSeconds)
	{
		NextFlushSeconds = FPlatformTime::Seconds() + Client->GetFlushIntervalSeconds();
		Flush();
	}
	return true;
}

void UChillSubsystem::Shutdown()
{
	if (CurrentRequest.IsValid())
	{
		CurrentRequest->CancelRequest();
		CurrentRequest.Reset();
	}
	bFlushing = false;
	if (Client.IsValid())
	{
		Client->Stop();
		Client.Reset();
	}
}
