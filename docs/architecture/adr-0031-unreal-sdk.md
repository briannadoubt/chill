# ADR-0031: Native C++ Unreal Engine SDK

- Status: Accepted
- Date: 2026-09-07
- Scope ticket: CHILL-129

## Context

Unreal games need the same semantic behavior contract the Unity, Swift, Android,
web, and portable SDKs already provide. Binding Unreal to the portable Rust
client would require shipping a native static library per target — Win64, Mac,
Linux, Android, iOS, and each console — and would put FFI lifetime and marshaling
behavior on the game thread. It would also give the plugin a binary supply-chain
surface that cannot be audited from the repository.

Unreal actor names, object paths, asset paths, transforms, Blueprint pin values,
and log text are not stable or privacy-safe semantic inputs. The integration must
not infer telemetry from them, for the same reasons recorded in
[ADR-0030](adr-0030-unity-sdk.md).

## Decision

Chill ships `sdk/unreal` as a source-only Unreal plugin targeting Unreal Engine
5.4 and newer. It depends only on first-party engine modules — `Core`,
`CoreUObject`, `Engine`, and `HTTP` — and contains no precompiled binaries and no
content assets.

The plugin provides:

- a consent-gated C++ client for sessions, pages, impressions, actions, events,
  and paired activities, mirroring the Unity surface;
- a bounded annotation allow-list with privacy classifications carried on the
  wire, where an ancestor's value wins on collision;
- a crash-safe at-least-once durable queue and an OTLP/HTTP JSON exporter driven
  by a `UGameInstanceSubsystem`.

The envelope is byte-compatible with the other SDKs apart from its identity
attributes: `telemetry.sdk.language` is `cpp`, `process.runtime.name` and
`chill.game.engine` are `unreal`, and the scope is `dev.chill.unreal`.

### The plugin does not declare `EngineVersion`

A `.uplugin` `EngineVersion` field pins a plugin to one exact engine build, and
newer engines refuse to load it. That is correct for a prebuilt marketplace
plugin and wrong for a source plugin that supports a minimum version. The
supported minimum is recorded in `ci/toolchains.json` and the integration guide
instead, and `scripts/validate-unreal-sdk.py` fails the build if the field
reappears.

### Credentials never enter the queue

`IChillStore` and its implementations have no access to the configuration. The
subsystem attaches `Authorization: Bearer <key>` when it sends, so a queue file
recovered from disk cannot carry a credential. The validation script enforces
this by rejecting the tokens `SdkKey`, `Authorization` and `Bearer` anywhere in
the store sources.

## Consequences

GitHub-hosted runners have no Unreal Engine installation, so CI validates the
source contract only: `scripts/validate-unreal-sdk.py` checks the package
boundaries, the module and dependency surface, the absence of binaries, the
transport and client invariants, and the release integration. This matches the
`source-contract-plus-local-editor` status already recorded for Unity.

Compilation and the automation suite are verified locally against an installed
engine. The initial implementation was built and run against Unreal Engine 5.8.2
on macOS for the Editor, Development and Shipping targets, and all 14
`Chill.Unreal.*` automation tests pass.

Wire compatibility is not left to inspection. The automation test
`Chill.Unreal.Fixture.DumpEnvelope` writes an envelope through the real client;
that capture is committed as `backend/testdata/unreal-sdk-otlp.json` with only
the volatile identifiers pinned, and
`backend/crates/chill-normalize/tests/unreal_sdk.rs` replays it through the
production normalizer and validates every record against the behavior schema.
That test runs on hosted CI, so the envelope contract is enforced even though the
plugin itself is not compiled there.

Because the plugin is source-only, integrators compile it with their own game and
can audit every line that runs in their process.
