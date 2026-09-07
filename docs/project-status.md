# Project status

Chill is an early alpha open-source project. The repository contains a working
cross-platform contract, native SDK implementations, a Rust backend, a console,
deployment assets, and extensive automated validation. It is not yet a hosted
service or a production support offering.

## Maturity

| Area | Current state |
| --- | --- |
| Canonical behavior and privacy contracts | Implemented and versioned |
| Rust backend | Working modular monolith; PostgreSQL and S3-compatible storage required |
| Swift SDK | Complete first vertical slice with Apple-specific validation |
| Android SDK | Active implementation with contract, lint, test, and budget gates |
| Browser SDK | Active implementation with contract, test, and bundle gates |
| Unity SDK | Active UPM package with Unity Test Framework coverage, declarative components, and bounded OTLP delivery |
| Unreal SDK | Active source-only UE 5.4+ plugin with automation-test coverage, consent-gated capture, and bounded OTLP delivery |
| Portable Rust SDK | Active Linux implementation with contract, package, and cross-target gates |
| Node, Deno, and Bun SDK | Active ESM runtime with pinned compatibility and package gates |
| Electron adapter | Active metadata-only IPC and lifecycle adapter; deterministic adapter coverage |
| Tauri 2 adapter | Active Rust host and webview correlation packages; deterministic adapter coverage |
| React console | Working client; authentication and hosting adapters are deployment-specific |
| Self-hosted deployment | Reproducible single-node and provider examples |
| Managed cloud service | Not offered |
| Compatibility promise | Pre-1.0; breaking changes may occur |

## Appropriate uses

Chill is appropriate for source review, local evaluation, prototypes, contract
experiments, and contributions. Self-hosting is possible for operators who are
comfortable owning PostgreSQL, object storage, secrets, upgrades, backups,
privacy policy, and incident response.

Do not assume an uptime SLA, stable API, migration window, compliance
certification, data-processing agreement, or security bounty. Review the threat
model, privacy policies, deployment guides, and source before handling real
user data.

## Release criteria

Releases use coordinated semantic versions and the gates in
`docs/releasing.md`. A 1.0 release requires stable public contracts,
documented upgrade compatibility, repeatable recovery evidence, supported
hosted CI for release-critical SDK checks, and a stated support policy.

Until then, pin exact versions or commit SHAs and test upgrades against your own
data and infrastructure.

The exact runtime and target matrix, including current non-goals, is maintained
in [platform support](platform-support.md).
