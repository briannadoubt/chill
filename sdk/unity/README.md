# Chill for Unity

`com.chill.observability` is a native C# Unity Package Manager SDK for
privacy-first semantic behavior instrumentation. It supports Unity 2022.3 LTS
and newer with Mono or IL2CPP and does not ship a native plug-in.

The package records only after application consent is granted. It accepts
bounded values through explicitly declared annotation keys, persists already
redacted records in an at-least-once queue, and sends OTLP/HTTP JSON to an exact
`/v1/logs` endpoint. The SDK key remains in memory and is never written to the
queue.

## Install

Use Package Manager's **Add package from git URL** command:

```text
https://github.com/briannadoubt/chill.git?path=/sdk/unity#v0.1.0
```

For a local checkout, choose `sdk/unity/package.json` with **Add package from
disk**.

## Configure

Configure once from your application's composition root. Obtain the SDK key
from app configuration; do not place it in a scene, prefab, or `ScriptableObject`.

```csharp
using System;
using Chill.Unity;

var configuration = new ChillConfiguration(
    "cats.game",
    new Uri("https://telemetry.example.com/v1/logs"),
    sdkKeyFromAppConfiguration)
{
    Consent = playerGrantedAnalytics
        ? ChillConsent.Granted
        : ChillConsent.Denied
};

configuration
    .AllowAnnotation("player.cohort", ChillAnnotationClassification.PseudonymousIdentifier)
    .AllowAnnotation("match.mode");

ChillRuntime.Configure(configuration);
```

Call `ChillRuntime.SetConsent(...)` whenever application consent changes.
Denial aborts the active request, purges queued records, closes the session,
and prevents new capture.

## Declare Unity behavior

- Add `Chill Scene Page` to a scene root and provide a stable lowercase segment
  such as `main_menu`.
- Add `Chill Semantic Context` to a parent object to declare bounded string
  annotations inherited by its children. Outermost values win collisions.
- Add `Chill Semantic Action` to a control and connect its `Activate()` method
  to the control's committed `UnityEvent`.
- Add `Chill Semantic Event` when an existing `UnityEvent` represents a stable
  domain transition.
- Add `Chill Semantic Impression` to content whose enabled lifecycle represents
  visibility.

Components never inspect object names, labels, input text, event arguments,
scene paths, or gameplay payloads.

For domain work, use an explicit activity scope and choose a stable reason code:

```csharp
ChillActivityScope activity = ChillRuntime.BeginActivity(
    "inventory.load",
    new ChillAnnotations().With("match.mode", "ranked"),
    ChillActivityKind.Storage);

try
{
    LoadInventory();
    activity.Succeed();
}
catch
{
    activity.Fail("inventory_unavailable");
    throw;
}
finally
{
    activity.Dispose();
}
```

Call `ChillRuntime.Flush()` at an explicit lifecycle drain when useful.
Periodic and application-background flushes are automatic. Queued records
survive unsuccessful responses and retain their original record IDs.

## Platform identity

Unity records preserve the canonical Chill platform family:

- Android player → `android`
- iOS/tvOS player → `apple`
- WebGL player → `web`
- desktop, console, editor, and dedicated server → `server`

OTLP resource attributes carry `process.runtime.name=unity`, the Unity version,
the operating-system family, and `chill.game.engine=unity`.
