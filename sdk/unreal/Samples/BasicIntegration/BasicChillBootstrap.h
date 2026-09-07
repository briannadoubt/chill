// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "BasicChillBootstrap.generated.h"

/**
 * Minimal integration sample.
 *
 * Configures Chill, declares the annotations the game is allowed to send, and
 * records one page and one action. Consent is left denied so the sample captures
 * nothing until the game grants it.
 */
UCLASS()
class UBasicChillBootstrap : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;

	/** Call once the player has accepted the privacy prompt. */
	void OnPlayerGrantedConsent();
};
