# Apple physical release validation

This fixture builds two optimized iOS applications from the same source:

- `ChillPhysicalBaseline` compiles Chill out entirely.
- `ChillPhysicalCandidate` links core and replay and uses declarative instrumentation.

The fixture exercises SwiftUI navigation, scrolling, forms, animation, custom
drawing, WebKit, privacy canaries, queue bursts, and native UIKit controls. The
UI test target also verifies ten activation cycles for the UIKit button,
switch, segmented control, slider, and text field.

Regenerate the checked Xcode project after editing `project.yml`:

```sh
xcodegen generate --spec validation/apple/physical/project.yml
```

## Safe plan mode

Plan mode validates the budget catalog and manifest, then prints a deterministic
schedule without contacting a device or collecting data:

```sh
python3 scripts/run-apple-release-validation.py \
  --seed 913 \
  --output /tmp/apple-release-plan.json
```

The same seed and inputs always produce the same plan and execution digest.
Each paired trial keeps baseline and candidate adjacent while randomizing their
order, so sequential variant batches are impossible.

## Physical execution

Execution requires an attached, paired iPhone XS, XS Max, or XR running iOS
17.0.x. The fixture fails closed unless the device is physical, the app reports
nominal thermal state, the phone reports unplugged power, both variants use
Release configuration, and both environment-preflight tests actually execute
rather than skip.

```sh
python3 scripts/run-apple-release-validation.py \
  --seed 913 \
  --execute \
  --device "$CHILL_APPLE_DEVICE_ID" \
  --team "$CHILL_APPLE_DEVELOPMENT_TEAM" \
  --output /tmp/apple-release-execution.json
```

The runner executes every XCTest-backed task in the exact planned order and
writes unique raw result bundles under `evidence/`. App-thinning, Instruments,
Power Profiler, and measurement extraction remain named pending collectors.
Execution therefore reports `incomplete` and `measurements_collected: 0`; a
successful test run is never converted into invented budget values.

The shorter smoke command remains useful for verifying signing and interaction
before reserving the device for the multi-hour release suite:

```sh
scripts/run-apple-physical-smoke.sh \
  --device "$CHILL_APPLE_DEVICE_ID" \
  --team "$CHILL_APPLE_DEVELOPMENT_TEAM"
```

## Observation fragments and aggregation

Collector output is represented by JSON fragments bound to the plan and raw
artifacts. Each fragment contains:

- `profiler_manifest_sha256`, `budget_catalog_sha256`,
  `execution_plan_sha256`, and `interleaving_seed`;
- `build` with distinct candidate/baseline identities, revision, Release
  configuration, and Xcode/Swift versions;
- `device` with class, model, OS, physical-device flag, thermal state, and
  power state;
- `artifacts` with a sanitized name, local path, and verified SHA-256;
- `observations` containing either scalar `values` or ordered
  `{baseline, candidate}` `pairs`.

Aggregate complete fragments with:

```sh
python3 scripts/aggregate-apple-release-report.py \
  --manifest validation/apple/v1/profiler-scenarios.json \
  --output /tmp/apple-replay-report.json \
  validation/apple/physical/evidence/fragments
```

Aggregation fails on any missing one of the 40 `apple.replay` measurements,
insufficient samples, wrong unit/statistic/method, missing required artifact,
artifact digest mismatch, mixed plan/build provenance, non-floor physical
device, non-nominal thermal state, or plugged power. The sanitized report omits
device IDs, signing teams, local paths, provisioning details, and artifact
names. It can then be evaluated with:

```sh
scripts/validate-swift-sdk.sh --release-report /tmp/apple-replay-report.json
```

`evidence/` is intentionally ignored because Xcode logs, result bundles, and
collector output can contain private device and signing values. `attestations/`
contains only sanitized, reviewable evidence.

The 2026-07-15 smoke attestation is diagnostic only: its variants were sampled
in sequential batches, its phone was not on the floor OS, and required thermal
and unplugged-power provenance was absent. It must continue to fail release
evaluation and must never be transformed into a release budget report.
