# Releasing Chill

Chill uses a coordinated repository version and publishes only from a stable
tag. Ordinary development uses the prerelease value in `VERSION` and cannot
publish production artifacts.

## Prepare a release

1. Choose a stable SemVer version and replace the contents of `VERSION`.
2. Regenerate every contract-major integrity manifest:

   ```sh
   python3 scripts/repository_contract.py update --contract-major 1
   python3 scripts/repository_contract.py update --contract-major 2
   ```

3. Run the repository, contract, backend, Rust, JavaScript runtime, Electron,
   Tauri, Swift, simulator, and required physical-device release gates. The
   physical Apple report must satisfy the catalog in
   `validation/apple/v1/profiler-scenarios.json`; portable reports must bind the
   V2 suite digest and satisfy `budgets/sdk/v2/budgets.json`.
4. Merge the reviewed release-preparation change only after CI succeeds.
5. Create and push an annotated tag that exactly matches the version:

   ```sh
   version="$(tr -d '\n' < VERSION)"
   python3 scripts/repository_contract.py verify --release-tag "v$version"
   git tag -a "v$version" -m "Chill $version"
   git push origin "v$version"
   ```

The tag starts `.github/workflows/release.yml`. Configure the GitHub
`production` environment with required reviewers before the first external
release. Do not upload artifacts or images by hand; a failed workflow is not a
partial release and should be corrected with a new commit and version.

## Published artifacts

The workflow publishes:

- `chill-swift-<version>.tar.gz`;
- `chill-web-<version>.tar.gz`;
- `chill-android-<version>.tar.gz`;
- `chill-rust-<version>.tar.gz`;
- `chill-javascript-<version>.tar.gz`, containing the runtime, Electron, and Tauri TypeScript packages;
- `chill-tauri-<version>.tar.gz`, containing the Rust host adapter;
- `chill-contracts-<version>.tar.gz`;
- `manifest.json`, containing commit, size, and SHA-256 identity; and
- a multi-platform backend image in GHCR.

The file artifacts and OCI digest receive GitHub artifact attestations. Verify
a downloaded artifact against this repository:

```sh
gh attestation verify chill-swift-<version>.tar.gz --repo <owner>/<repo>
```

Inspect and deploy the backend by the digest emitted by the release workflow,
not by a mutable version tag.

## Adding a component

When a planned component becomes real, update `ci/components.json` from
`planned` to `active` in the same change that adds its source path. Add its
toolchain, pull-request gate, deterministic release artifact, provenance step,
and owner. Repository verification deliberately rejects source that appears
before this activation contract is complete.
