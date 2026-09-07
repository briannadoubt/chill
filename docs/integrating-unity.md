# Integrating Unity

Chill for Unity is a source-only Unity Package Manager package for Unity 2022.3
LTS and newer. It compiles under Mono or IL2CPP and has no native library to
copy into player builds.

## Install the package

In Package Manager, choose **Add package from git URL** and enter:

```text
https://github.com/briannadoubt/chill.git?path=/sdk/unity#v0.1.0
```

For a local checkout, use **Add package from disk** and select
`sdk/unity/package.json`.

## Configure once

Configure Chill in the application composition root before declared scene
components become active:

```csharp
using System;
using Chill.Unity;

var configuration = new ChillConfiguration(
    "cats.game",
    new Uri("https://telemetry.example.com/v1/logs"),
    sdkKeyFromAppConfiguration)
{
    Consent = analyticsConsent
        ? ChillConsent.Granted
        : ChillConsent.Denied,
    PolicyVersion = "privacy-v1"
};

configuration
    .AllowAnnotation(
        "player.cohort",
        ChillAnnotationClassification.PseudonymousIdentifier)
    .AllowAnnotation("match.mode");

ChillRuntime.Configure(configuration);
```

The endpoint must be HTTPS and end at the exact path `/v1/logs`; loopback HTTP
is allowed for local development. The SDK key stays in memory. Do not serialize
it into a scene, prefab, `ScriptableObject`, or queue file.

Collection starts denied unless the application explicitly grants it. Apply
later decisions with:

```csharp
ChillRuntime.SetConsent(
    analyticsConsent ? ChillConsent.Granted : ChillConsent.Denied);
```

Denial aborts an active upload, purges the at-least-once queue, closes the
current session, and makes declarations no-ops. Re-granting opens a new
session.

## Declare scene and object semantics

Add the following components from the **Chill** component menu:

- **Scene Page** on a scene root, with a stable segment such as `main_menu`.
  Chill does not derive a name from the Unity scene name or path.
- **Semantic Context** on an ancestor to declare allowlisted bounded string
  annotations inherited by children. The outermost existing value wins.
- **Semantic Action** on a control. Connect `Activate()` to the control's
  committed `UnityEvent`, not its pointer-down preview.
- **Semantic Event** where an existing `UnityEvent` represents a domain
  transition; connect `Observe()`.
- **Semantic Impression** on content whose enabled lifetime represents
  visibility.

These components do not inspect object names, labels, input text, UnityEvent
arguments, scene paths, or gameplay payloads.

## Instrument domain work

Use an activity scope for meaningful work that needs duration, outcome, and
trace identity:

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

Only stable reason codes enter telemetry. Exceptions, messages, stack traces,
arguments, and return values are not captured. Disposing an unfinished scope
marks it cancelled rather than guessing success.

Direct event declarations are available when no existing UnityEvent owns the
transition:

```csharp
ChillRuntime.Event(
    "match.completed",
    new ChillAnnotations().With("match.mode", "ranked"),
    ChillEventClass.Domain,
    ChillEventSeverity.Info);
```

There is deliberately no arbitrary-properties tracker. Annotation values must
be bounded primitives whose keys were declared in configuration or by a
Semantic Context component.

## Delivery and lifecycle

Chill starts and ends the application session, records foreground/background
lifecycle events, retains unsuccessful batches with their original UUIDv7
record IDs, and flushes periodically. Call `ChillRuntime.Flush()` for an
explicit lifecycle drain. A failed or interrupted request remains queued for a
later attempt; only a 2xx response is acknowledged.

The queue defaults to `Application.persistentDataPath/Chill/Queue`, is capped at
64 MiB, and contains only already-redacted OTLP log records. WebGL persistence
uses Unity's browser-backed persistent filesystem and therefore inherits the
browser's storage and eviction behavior.

Call `ChillRuntime.Shutdown()` when an application-controlled host lifecycle
can shut down before Unity's own quit callback. The session-end record is
durable even when the process cannot finish a final network request.

## Platform identity

The canonical `chill.source.platform` remains compatible with the shared
contract:

| Unity player | Canonical family |
| --- | --- |
| Android | `android` |
| iOS and tvOS | `apple` |
| WebGL | `web` |
| Desktop, editor, dedicated server, and console | `server` |

Resource attributes identify the real runtime with
`process.runtime.name=unity`, the Unity editor/player version, operating-system
family, and `chill.game.engine=unity`.

## Run the package tests

Add the package to a Unity test project, include it in the project's
`testables` list, and run EditMode tests:

```sh
Unity \
  -batchmode \
  -nographics \
  -projectPath /path/to/project \
  -runTests \
  -testPlatform EditMode \
  -testResults TestResults.xml
```

The package tests cover denied-consent no-op behavior, purge on denial,
annotation allowlisting and precedence, UUID versions, paired activities,
structured pages, endpoint restrictions, and at-least-once acknowledgement.
