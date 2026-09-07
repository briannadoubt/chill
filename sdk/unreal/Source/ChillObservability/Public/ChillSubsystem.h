// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"
#include "ChillClient.h"
#include "Containers/Ticker.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "ChillSubsystem.generated.h"

class IHttpRequest;

/**
 * Owns the client for the lifetime of the game instance and drives periodic flushes.
 *
 * The subsystem is the only place the SDK key is read: it attaches authorization to
 * the outgoing request, so the durable queue never stores a credential.
 */
UCLASS()
class CHILLOBSERVABILITY_API UChillSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	/** Creates and starts the client. Returns false when the configuration is rejected. */
	bool Configure(const FChillConfiguration& Configuration);

	bool IsConfigured() const { return Client.IsValid(); }
	FChillClient* GetClient() const { return Client.Get(); }

	void SetConsent(EChillConsent Consent);
	void Flush();
	void Shutdown();

private:
	bool Tick(float DeltaSeconds);
	void SendBatch(const FChillExportBatch& Batch);

	TUniquePtr<FChillClient> Client;
	TSharedPtr<IHttpRequest, ESPMode::ThreadSafe> CurrentRequest;
	FTSTicker::FDelegateHandle TickHandle;
	double NextFlushSeconds = 0.0;
	bool bFlushing = false;
};
