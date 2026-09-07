# Chill

[![CI](https://github.com/briannadoubt/chill/actions/workflows/ci.yml/badge.svg)](https://github.com/briannadoubt/chill/actions/workflows/ci.yml)
[![Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Project status: alpha](https://img.shields.io/badge/status-alpha-orange.svg)](docs/project-status.md)

Chill is an open-source, privacy-first behavior intelligence platform built
around one rule: application developers declare semantic context, but never
imperatively emit tracking events.

> **Early alpha:** Chill is pre-1.0. APIs, schemas, and operational procedures
> may change. It is suitable for evaluation and contributor development, not a
> production support commitment. Read the [project status](docs/project-status.md)
> before deploying it.

The repository includes:

- idiomatic SDKs for Apple, Android, browsers, Unity, Unreal, Rust, JavaScript runtimes, and desktop hosts;
- Swift macros for activities and domain events;
- automatic UI, navigation, network, performance, crash, and lifecycle capture;
- OpenTelemetry-compatible distributed tracing;
- privacy-safe structural session replay;
- a frugal Postgres, Parquet, object-storage, and DuckDB data platform; and
- analytics for live debugging, journeys, funnels, cohorts, retention, and
  dashboards;
- a Rust modular-monolith backend backed by PostgreSQL, Parquet, S3-compatible
  object storage, and embedded DuckDB; and
- a React console plus Swift, Android, and browser SDKs.

## Start here

| Goal | Guide |
| --- | --- |
| Understand maturity and support | [Project status](docs/project-status.md) |
| Integrate an Apple app | [Swift integration](docs/integrating-swift.md) |
| Integrate a browser app | [Web integration](docs/integrating-web.md) |
| Integrate an Android app | [Android integration](docs/integrating-android.md) |
| Integrate a Unity game | [Unity integration](docs/integrating-unity.md) |
| Integrate an Unreal game | [Unreal integration](docs/integrating-unreal.md) |
| Integrate a Rust or Linux process | [Rust integration](docs/integrating-rust.md) |
| Integrate Node, Deno, or Bun | [JavaScript runtime integration](docs/integrating-javascript-runtimes.md) |
| Integrate an Electron app | [Electron integration](docs/integrating-electron.md) |
| Integrate a Tauri app | [Tauri integration](docs/integrating-tauri.md) |
| Check exact target coverage | [Platform support](docs/platform-support.md) |
| Run the Rust backend locally | [Backend guide](backend/README.md) |
| Run a single-node deployment | [Operator guide](deploy/single-node/README.md) |
| Contribute | [Contribution guide](CONTRIBUTING.md) |

The contract starts with the
[canonical behavior model](docs/architecture/adr-0001-canonical-behavior-model.md),
[outer-first annotation resolution](docs/architecture/adr-0002-outer-first-annotation-resolution.md),
[structured page navigation](docs/architecture/adr-0003-structured-page-navigation.md),
[declarative actions, activities, and events](docs/architecture/adr-0004-declarative-actions-activities-events.md),
the [OpenTelemetry mapping and propagation contract](docs/architecture/adr-0005-opentelemetry-mapping-and-propagation.md),
[release-blocking SDK performance and privacy budgets](docs/architecture/adr-0006-sdk-performance-and-privacy-budgets.md),
the [exact cross-platform conformance suite](docs/architecture/adr-0007-cross-platform-conformance.md),
and the [Swift core runtime](docs/architecture/adr-0008-swift-core-runtime.md).
The first developer-facing instrumentation surface is defined by the
[Swift activity and event macros](docs/architecture/adr-0009-swift-activity-event-macros.md)
and the
[declarative SwiftUI adapter](docs/architecture/adr-0010-declarative-swiftui-instrumentation.md),
plus the
[automatic UIKit adapter](docs/architecture/adr-0011-automatic-uikit-instrumentation.md),
and the
[Apple networking trace adapter](docs/architecture/adr-0012-apple-networking-trace-propagation.md).
Durable delivery is defined by the
[Swift offline buffer and OTLP exporter](docs/architecture/adr-0013-swift-offline-buffer-and-otlp-export.md).
Privacy-safe playback is defined by the
[Apple structural session replay adapter](docs/architecture/adr-0014-privacy-safe-apple-session-replay.md).
The service implementation and its measured extraction boundaries are defined
by the
[Rust server-platform decision](docs/architecture/adr-0026-rust-server-platform.md).
Portable Rust, JavaScript runtime, Electron, and Tauri boundaries are defined
by the
[portable SDK architecture](docs/architecture/adr-0029-portable-rust-and-desktop-sdks.md).
Unity's source-only C# and player-target boundary is defined by the
[Unity SDK architecture](docs/architecture/adr-0030-unity-sdk.md).
Unreal's source-only C++ and engine-module boundary is defined by the
[Unreal SDK architecture](docs/architecture/adr-0031-unreal-sdk.md).
Consent changes, remote collection restrictions, privacy exports, and deletion
fulfillment are defined by the
[auditable privacy workflow](docs/architecture/adr-0024-consent-export-and-deletion-workflows.md).
Start an application integration with the copyable, compile-checked
[Apple integration guide](docs/integrating-swift.md).

## Swift declaration model

Application code imports the small `Chill` library and declares behavior on
the function that owns it:

```swift
import Chill

@Activity("repository.load_cat", kind: .storage)
func loadCat(id: Cat.ID) async throws -> Cat {
  try await repository.cat(id: id)
}

@Event(
  "cat.adopted",
  severity: .info,
  emission: .succeeded,
  deduplicationKey: adoptionID
)
func completeAdoption(adoptionID: String) async throws {
  try await adoption.complete()
}
```

There is no tracking call in either body. The attached macros preserve the
function declaration and instrument its entered or terminal lifecycle.

SwiftUI behavior is declared where its semantic meaning is known:

```swift
NavigationStack {
  VStack {
    ChillActionButton("Adopt", action: "cat.adopt") {
      adopt()
    }

    ChillToggle("Favorite", action: "cat.favorite", isOn: $favorite)

    Text("Hero")
      .impression("cat.hero", role: "heading")
  }
  .annotation("cat.id", value: cat.id)
  .event("cat.favorite.changed", when: favorite)
  .page("cat")
  .privacy(.redacted)
  .replay(.masked)
}
.page("app")
```

Nested pages produce structured paths such as `app / cat`; identifiers stay in
bounded annotations. Later, structurally outer annotation declarations win a
collision. Native Chill controls record committed actions without inspecting
labels or values, while a generic `.action` remains available for tap and
submit declarations on existing views.

UIKit uses the same declaration vocabulary during normal view setup. Native
containers and controls emit automatically afterward:

```swift
final class CatViewController: UIViewController {
  override func viewDidLoad() {
    super.viewDidLoad()

    annotation("cat.id", value: catID)
    page("cat")

    adoptButton.action("cat.adopt")
    heroView.impression("cat.hero", role: "image")
  }
}

let navigation = UINavigationController(rootViewController: catController)
  .annotation(["account.tier": "internal"])
  .page("app")
```

The adapter diffs navigation stacks, tabs, split columns, presentations, and
scene lifecycle per window. It does not globally swizzle UIKit or replace app
delegates. Optional native `ChillButton`, `ChillSwitch`, picker/adjustment, and
text-field subclasses also carry annotation and page context through target
dispatch so Swift macros invoked by callbacks correlate automatically.

First-party network propagation is configured once on an existing URLSession.
Native action callbacks and `@Activity` functions establish task-local trace
context automatically, so requests need no trace identifiers or tracking calls:

```swift
let policy = try ChillNetworkPropagationPolicy(
  trustedOriginURLs: [URL(string: "https://api.example.com")!],
  baggageAllowlist: ["release.channel"],
  baggage: ["release.channel": "internal"]
)
let network = URLSession.shared.chill(policy)

@Activity("repository.load_cat", kind: .network)
func loadCatRequest(_ request: URLRequest) async throws -> Data {
  try await network.data(for: request).0
}
```

Trust is exact-origin and is re-evaluated on every redirect. Baggage is empty by
default and never derives from annotations or identity. Network spans omit URL
paths, queries, headers, bodies, and error descriptions.

Configure durable OTLP delivery once at application startup. All declarative UI
facts and macro-generated activity/event facts flow into this sink
automatically:

```swift
let applicationSupport = FileManager.default.urls(
  for: .applicationSupportDirectory,
  in: .userDomainMask
)[0]
let pipeline = try ChillOfflinePipeline(
  directory: applicationSupport.appending(path: "Chill/Queue"),
  configuration: ChillOTLPConfiguration(
    endpoint: URL(string: "https://telemetry.example.com/v1/logs")!,
    headers: ["Authorization": "Bearer <project credential>"],
    resourceAttributes: ["service.name": "cats-ios"]
  )
)
let runtime = ChillRuntime(
  configuration: runtimeConfiguration,
  sink: pipeline
)
Chill.configure(runtime)
```

The default queue is capped at 64 MiB, writes atomically with checksum recovery,
evicts replay and impressions before high-value facts, batches OTLP protobuf,
and uses gzip plus jittered retry. Credentials remain memory-only. Call
`await pipeline.flush()` only for an explicit lifecycle drain or test; normal
timed and threshold delivery is automatic.

An authenticated collection-state client can refresh the running SDK at launch
and foreground transitions. Local application consent grants capture; the
server may only narrow it, and stale revisions or privacy-policy mismatches fail
closed:

```swift
let collection = try ChillCollectionPolicyClient(
  endpoint: URL(string: "https://telemetry.example.com/v1/chill/collection-state")!,
  headers: ["Authorization": "Bearer <project credential>"]
)
let policies = ChillCollectionPolicySynchronizer(
  runtime: runtime,
  client: collection
)
try await policies.refresh()
```

Session replay is a separate product and a separate consent decision. Configure
it once after the runtime; the native adapter then observes structural changes,
viewport and scroll state, and gestures without screenshots or tracking calls:

```swift
import ChillReplay

let replayConfiguration = try ChillReplayConfiguration.standard(
  directory: applicationSupport.appending(path: "Chill/Replay"),
  keyIdentifier: "cats-internal"
)
try ChillSessionReplay.configure(replayConfiguration)
```

Replay defaults to off through the runtime's independent replay consent and
sampling gates. Text becomes a length-bucket mask, secure inputs are always
masked, pixels/web/media/custom drawing become opaque rectangles, and SwiftUI
`.privacy` / `.replay` boundaries are enforced by zero-content native probes.
Keyframe-plus-delta chunks are gzip-compressed, AES-GCM encrypted with a
device-local Keychain key, and capped at 64 MiB by default. Only correlated
chunk metadata enters OTLP; encrypted replay payloads use their own upload and
acknowledgement path.

## Validate the contract

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements-contract.txt
.venv/bin/python scripts/validate-contract-fixtures.py
python3 scripts/run-conformance-suite.py
python3 -m unittest discover -s tests -p 'test_*.py'
scripts/test-swift-sdk.sh
scripts/run-swift-core-benchmarks.sh
scripts/run-swift-replay-benchmarks.sh
scripts/run-swift-conformance.sh /tmp/apple-conformance.json
scripts/validate-swift-sdk.sh
```

The unified Swift command builds the independent SwiftUI and UIKit consumer
samples on macOS and iOS, runs the host test suite, executes all nine shared
conformance domains, scans the resulting report for privacy canaries, and runs
the release-mode core and replay presubmit benchmarks. The sample source lives
in [`sdk/swift/Examples`](sdk/swift/Examples).

Host and simulator numbers are deliberately labeled presubmit-only. A release
candidate must also supply the checked physical-device profiler scenarios in
[`validation/apple/v1/profiler-scenarios.json`](validation/apple/v1/profiler-scenarios.json):

```sh
scripts/validate-swift-sdk.sh --release-report apple-device-report.json
```

The backend control plane has an equivalent PostgreSQL integration and restore
gate documented in [`backend/README.md`](backend/README.md).
The complete frugal production slice can be run on one digest-pinned Docker host;
see the [single-node operator guide](deploy/single-node/README.md) and
[deployment decision](docs/architecture/adr-0021-reproducible-single-node-deployment.md).

Repository boundaries, coordinated versioning, CI matrices, dependency updates,
and signed release provenance are defined by the
[monorepo and release decision](docs/architecture/adr-0025-monorepo-ci-and-release.md).
Run `python3 scripts/repository_contract.py verify` before review. Stable release
preparation and attestation verification are documented in the
[release runbook](docs/releasing.md).

## Product constraints

- Swift ships as the first complete vertical slice.
- Swift, Android, and web ship platform-native shared-contract implementations.
- React Native is deferred.
- Flutter is out of scope.
- Annotation inheritance is root-authoritative: an outer value wins when the
  same key appears in an inner scope.
- Initial operation must remain practical for one developer on a small budget.

## Community and license

Chill is maintained in the open under the [Apache License 2.0](LICENSE).
Contributions are welcome under the [contribution guide](CONTRIBUTING.md) and
[Code of Conduct](CODE_OF_CONDUCT.md). Use [support guidance](SUPPORT.md) for
questions and [the security policy](SECURITY.md) for private vulnerability
reporting. Project decisions and maintainer responsibilities are described in
[GOVERNANCE.md](GOVERNANCE.md).
