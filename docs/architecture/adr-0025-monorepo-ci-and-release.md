# ADR-0025: Coordinated monorepo, CI, and release provenance

Status: accepted

## Decision

Chill remains one coordinated monorepo while the platform is pre-1.0 and
maintained by a small team. Contract definitions, native SDKs, the Rust backend,
deployment assets, validation applications, and future console code share one
review boundary and one repository version. This keeps a semantic-contract
change beside the conformance fixtures and every active implementation that
must adopt it.

`ci/components.json` is the machine-readable ownership and release registry.
Each component declares its paths, owners, versioning policy, lifecycle state,
and release artifact. Future components begin as `planned`; adding any of their
paths makes repository verification fail until the same change activates the
component and supplies its CI and release behavior. Splitting a component into
another repository requires a new ADR and a replacement mechanism that pins
the contract revision across repositories.

## Versioning and generated contracts

`VERSION` contains the coordinated SemVer value. Development commits may use a
prerelease version. A release tag must be exactly `v<VERSION>` and `VERSION`
must be stable SemVer, preventing a tag from silently publishing a differently
identified SDK or backend image.

The canonical contract is versioned independently through its declared
contract major. `contracts/generated/manifest.v1.json` binds every V1 tracked
JSON or Python input to its byte count and SHA-256 digest. Later contract-major
manifests, beginning with `manifest.v2.json`, contain only their versioned
extension inputs and cryptographically bind the unchanged V1 manifest. The
deterministic generator uses the Git index, not the untracked working tree, so
local exports, device artifacts, and editor copies cannot enter a release
accidentally. CI rejects any stale generated manifest.

## Reproducible builds and test matrix

`ci/toolchains.json` records the toolchains that build active components.
It pins the Rust production backend toolchain under ADR-0026.
Hosted jobs use an exact runner generation, exact language version, digest-
pinned Swift container, digest-pinned service images, and full-commit GitHub
Action references. A repository policy test rejects mutable Action references,
implicit workflow permissions, `pull_request_target`, mismatched toolchains,
and missing provenance steps.

Pull requests and the default branch run these independent gates:

- contract fixtures, generated-manifest integrity, conformance tests, and
  reproducible source-archive comparison;
- Rust unit, Clippy, formatting, Postgres/object-store integration, and OCI
  image build checks; and
- portable Rust/Linux, Node, Deno, Bun, Electron, and Tauri package,
  conformance, privacy, and compatibility checks; and
- Swift manifest parsing, dependency-lock resolution, source formatting, and a
  public imperative-API source guard in the digest-pinned Swift 6.4 Linux
  snapshot.

The Swift package currently requires Swift 6.4 and Apple releases require
Xcode 27. The hosted macOS runner catalog does not yet provide that toolchain.
The SDK also intentionally uses Apple-only runtime modules, so the Linux job is
not represented as an executable SDK test. Simulator, complete package, symbol,
and physical-device release validation remain an external, checked attestation
rather than a misleading hosted job. Once a compatible hosted runner exists,
`ci/toolchains.json` and this matrix must move the Apple gates into CI without
weakening the physical-device budget contract.

## Release and provenance

A stable `vX.Y.Z` tag is the only publishing trigger. The production environment
may require a reviewer in repository settings. The workflow re-verifies the
tag and generated contracts, then:

1. creates deterministic Swift, Android, browser, portable Rust, JavaScript,
   Tauri, and contract source archives from the tagged commit, plus a digest
   manifest;
2. produces signed GitHub artifact attestations for those files;
3. builds the backend image for Linux AMD64 and ARM64 using commit-derived
   build metadata;
4. pushes the image to GHCR by immutable digest and publishes signed registry
   provenance; and
5. creates the GitHub release only after every earlier step succeeds.

Consumers should verify a downloaded file with `gh attestation verify` and pin
production images by digest. A mutable image tag is a discovery aid, not a
deployment identity. The current development version cannot trigger a release;
the release preparation change must first replace it with a stable version and
pass the normal pull-request matrix.

## Dependency updates and ownership

CODEOWNERS makes the initial maintainer the explicit reviewer for every path.
Dependabot checks GitHub Actions, backend/portable/Tauri Cargo dependencies,
Swift packages, JavaScript packages, Python contract validation, Dockerfiles,
and Compose files weekly. Update concurrency is kept small and ecosystem
updates are grouped where practical so a one-developer project gets timely
supply-chain changes without an unmanageable pull-request queue. Every update
still passes the same repository and component matrix.

This is intentionally frugal: hosted CI and GHCR are used before introducing a
dedicated build platform, signing service, or package promotion system. The
registry and provenance contracts preserve a clean upgrade path when the team
or tenancy model grows.
