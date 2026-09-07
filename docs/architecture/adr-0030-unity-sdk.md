# ADR-0030: Native C# Unity SDK

- Status: Accepted
- Date: 2026-07-24
- Scope ticket: CHILL-128

## Context

Unity games need the same semantic behavior contract as Chill's native and web
SDKs, but a binding directly to the portable Rust client would require native
binaries for each editor, player, mobile, server, WebGL, and console target.
That would also put FFI lifetime, callback, and IL2CPP marshaling behavior on
the application hot path.

Unity object names, scene paths, UI labels, UnityEvent arguments, exception
messages, and gameplay values are not stable or privacy-safe semantic inputs.
The integration must not infer telemetry from them.

## Decision

Chill ships `sdk/unity` as a source-only Unity Package Manager package targeting
Unity 2022.3 LTS and newer. It has no native plug-in dependency and uses only
Unity engine modules plus .NET Standard APIs supported by Mono and IL2CPP.

The package provides:

- a consent-gated C# client for sessions, pages, impressions, actions, events,
  and paired activities;
- hierarchy components that declare static semantic names and bounded context,
  with outer-first annotation precedence;
- an explicit `Activate()`/`Observe()` bridge for committed UnityEvents;
- a capped file queue containing already-redacted OTLP log JSON;
- UUIDv7 record identity, UUIDv4 installation/process identity, and W3C trace
  identity; and
- `UnityWebRequest` OTLP/HTTP export that acknowledges only 2xx responses.

Credentials stay in the live configuration object and request header. They are
not serialized into scenes, prefabs, ScriptableObjects, queue entries, or
diagnostics. Consent denial aborts transport and purges the queue before any
new capture.

The V1 canonical source family is chosen from the Unity player target:
Android, Apple, web, or server. Unity identity is expressed in resource
attributes rather than by extending the canonical enum.

## Consequences

The package can be imported by Android, iOS, WebGL, desktop, dedicated-server,
and authorized console Unity projects without selecting native artifacts.
Console networking and persistent storage still depend on the platform
holder's Unity modules and policies.

Domain activities require an explicit scope because C# has no portable
attached-macro facility and Unity coroutines cannot safely share an ambient
activity stack. The scope never observes work arguments or results and defaults
an unfinished operation to cancelled.

The package does not yet provide structural session replay or automatic
network trace-header injection. Those require separate Unity-specific privacy,
render-pipeline, coroutine, and redirect decisions.
