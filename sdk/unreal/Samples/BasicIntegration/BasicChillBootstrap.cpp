// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "BasicChillBootstrap.h"

#include "ChillSubsystem.h"

void UBasicChillBootstrap::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);

	FChillConfiguration Configuration(
		TEXT("demo.game"),
		TEXT("https://collector.example.com/v1/logs"),
		TEXT("replace-with-your-sdk-key"));

	// Only declared annotations may ever be emitted.
	Configuration.AllowAnnotation(TEXT("game.mode"), EChillAnnotationClassification::Public);
	Configuration.AllowAnnotation(TEXT("game.level_index"), EChillAnnotationClassification::Internal);

	if (UChillSubsystem* Chill = GetGameInstance()->GetSubsystem<UChillSubsystem>())
	{
		Chill->Configure(Configuration);
	}
}

void UBasicChillBootstrap::OnPlayerGrantedConsent()
{
	UChillSubsystem* Chill = GetGameInstance()->GetSubsystem<UChillSubsystem>();
	if (Chill == nullptr || !Chill->IsConfigured())
	{
		return;
	}
	Chill->SetConsent(EChillConsent::Granted);

	FChillClient* Client = Chill->GetClient();
	const FChillAnnotations Annotations = FChillAnnotations()
		.With(TEXT("game.mode"), FString(TEXT("ranked")));

	Client->StartPage(TEXT("main-menu"), Annotations, EChillPageRelation::Root, EChillPageCause::Initial);
	Client->Action(TEXT("play"), Annotations, EChillActionActivation::Primary, EChillInput::Pointer);
}
