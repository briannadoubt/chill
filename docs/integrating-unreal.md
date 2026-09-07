# Integrating the Unreal Engine SDK

The Chill Unreal plugin captures semantic behavior and delivers it as OTLP/HTTP
JSON to the same ingest endpoint the other SDKs use. It ships as source, declares
no third-party dependencies, and contains no precompiled binaries.

## Requirements

- Unreal Engine **5.4** or newer. The plugin does not pin `EngineVersion`, so it
  loads on newer engines without being rebuilt for each release.
- A project that can compile C++ modules.

## Installing

Copy `sdk/unreal` into your project as `Plugins/Chill`:

```bash
cp -R sdk/unreal /path/to/YourGame/Plugins/Chill
```

Enable **Chill Observability** in the plugin browser, then add the module to the
game module that will call it:

```csharp
PublicDependencyModuleNames.AddRange(new string[] { "ChillObservability" });
```

## Configuring

Configure once, early. The endpoint must be an exact `/v1/logs` URL over HTTPS
(plain HTTP is accepted only for loopback development), and it may not carry
credentials, a query, or a fragment.

```cpp
#include "ChillSubsystem.h"

FChillConfiguration Configuration(
    TEXT("demo.game"),
    TEXT("https://collector.example.com/v1/logs"),
    TEXT("<sdk-key>"));

// Only declared keys may ever be emitted.
Configuration.AllowAnnotation(TEXT("game.mode"), EChillAnnotationClassification::Public);
Configuration.AllowAnnotation(TEXT("game.level_index"), EChillAnnotationClassification::Internal);

UChillSubsystem* Chill = GetGameInstance()->GetSubsystem<UChillSubsystem>();
Chill->Configure(Configuration);
```

## Consent

Capture is denied until you grant it, and revoking consent purges the durable
queue and cancels any in-flight upload.

```cpp
Chill->SetConsent(EChillConsent::Granted);  // after the player accepts
Chill->SetConsent(EChillConsent::Denied);   // discards everything captured
```

## Recording behavior

```cpp
FChillClient* Client = Chill->GetClient();

// Discrete interactions.
Client->Action(TEXT("play"), Annotations, EChillActionActivation::Primary, EChillInput::Pointer);
Client->Event(TEXT("match.started"), Annotations, EChillEventClass::Domain);
Client->Impression(TEXT("daily-reward"), Annotations, TEXT("content"), 1.0);

// Operations with an outcome and a duration.
{
    FChillActivityScope Activity =
        Client->BeginActivity(TEXT("level.load"), Annotations, EChillActivityKind::Task);
    Activity.Succeed();  // or Fail / Cancel / Timeout
}

// Navigation. The scope ends the page when it leaves scope.
{
    FChillPageScope Page = Client->StartPage(TEXT("main-menu"), Annotations);
    // Records emitted here carry the page path as context.
}
```

Ending an activity is idempotent: the first terminal call wins, and destruction of
an already-ended scope records nothing.

## Naming rules

- Semantic names are lowercase, at most 128 characters, and may contain `.`, `_`
  or `-` between segments: `main-menu`, `app.session`, `level.load`.
- Annotation keys are lowercase dotted identifiers: `game.mode`,
  `game.level_index`.

Anything that fails these rules is dropped with a diagnostic rather than sent.

## What the plugin will not do

- It never reads an actor name, transform, or asset path. Every identifier on the
  wire is one you passed explicitly.
- It drops annotations that were not declared through `AllowAnnotation`, so an
  undeclared value cannot reach the collector.
- The durable queue never stores the SDK key. The subsystem attaches the
  `Authorization` header at send time.
- Records leave the queue only after the collector accepts them, so a failed
  flush is retried rather than dropped.

## Diagnostics

`FChillClient::GetDiagnostics()` returns the most recent bounded diagnostics —
`annotation_disallowed`, `queue_full`, `flush_failed`, `invalid_name` and friends.
They are the supported way to see what was dropped and why.

## Verification

The plugin is validated three ways:

- `scripts/validate-unreal-sdk.py` enforces the package boundaries in CI.
- 14 automation tests under `Chill.Unreal.*` cover the capture contract; run them
  with `UnrealEditor-Cmd <project> -ExecCmds="Automation RunTests Chill.Unreal; Quit"`.
- `backend/crates/chill-normalize/tests/unreal_sdk.rs` replays a captured envelope
  through the production normalizer and validates it against the behavior schema.

Hosted CI runs the source contract only; compiling against the engine requires a
local Unreal installation, matching how the Unity package is validated.
