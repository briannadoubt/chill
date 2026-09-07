# Chill Observability for Unreal Engine

Privacy-first semantic behavior instrumentation and OTLP delivery for Unreal Engine.

The plugin ships as source. It declares no third-party dependencies and contains no
precompiled binaries, so it can be audited and built from the repository as-is.

## Installing

Copy `sdk/unreal` into your project as `Plugins/Chill`, then enable **Chill
Observability** in the plugin browser. Add `ChillObservability` to your module's
`PublicDependencyModuleNames`.

## Capturing behavior

```cpp
#include "ChillSubsystem.h"

FChillConfiguration Configuration(
    TEXT("demo.game"),
    TEXT("https://collector.example.com/v1/logs"),
    TEXT("<sdk-key>"));
Configuration.AllowAnnotation(TEXT("game.mode"), EChillAnnotationClassification::Public);

UChillSubsystem* Chill = GetGameInstance()->GetSubsystem<UChillSubsystem>();
Chill->Configure(Configuration);

// Capture stays off until the player grants consent.
Chill->SetConsent(EChillConsent::Granted);

FChillClient* Client = Chill->GetClient();
Client->Action(TEXT("play"), FChillAnnotations(), EChillActionActivation::Primary);
```

## What the plugin will not do

- It never captures a name, transform, or asset path automatically. Every recorded
  identifier is one you passed explicitly.
- It refuses annotations that were not declared through `AllowAnnotation`, so an
  undeclared value cannot reach the wire.
- Capture is denied until consent is granted, and revoking consent purges the
  durable queue.
- The durable queue never stores the SDK key; authorization is attached by the
  transport at send time.

See [docs/integrating-unreal.md](../../docs/integrating-unreal.md) for the full
integration guide and [ADR 0031](../../docs/architecture/adr-0031-unreal-sdk.md)
for the design rationale.
