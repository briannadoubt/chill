# Integrating Chill on Apple platforms

This is the implementation guide for Chill's Swift-first SDK. It covers the
supported integration surface as it exists today: one runtime configuration,
declarative SwiftUI or UIKit semantics, Swift macros, durable OTLP/HTTP Logs,
first-party W3C trace propagation, remote collection restrictions, and
privacy-safe structural replay.

There is no public `track`, `record`, or `capture` call. Product code declares
meaning where it already owns that meaning; native controls, navigation
adapters, macros, and the runtime produce records at the committed boundary.

## Requirements and installation

The current package requires Swift 6.4 and supports iOS 17, macOS 14, tvOS 17,
watchOS 10, and visionOS 1 or newer. Xcode 27 is the checked Apple toolchain.

During internal development, add `sdk/swift` as a local Swift package. After
the repository is hosted, add its Git URL in Xcode and select an exact stable
release. Applications normally link the `Chill` product. Link `ChillReplay`
only when the product has separately approved session replay.

```swift
import Chill
import SwiftUI
```

`Chill` re-exports the core runtime, durable exporter, networking adapter,
SwiftUI adapter, Swift macros, and UIKit adapter where UIKit is available. It
does not install another analytics or OpenTelemetry SDK.

The runnable [SwiftUI sample](../sdk/swift/Examples/Sources/ChillSwiftUISample/ChillSwiftUISample.swift)
and [UIKit sample](../sdk/swift/Examples/Sources/ChillUIKitSample/ChillUIKitSample.swift)
are consumer-package builds, not pseudocode. The shared
[sample bootstrap](../sdk/swift/Examples/Sources/ChillSampleSupport/SampleChill.swift)
configures the same public runtime used below.

## Configure the runtime once

Configure Chill before the first instrumented scene or controller appears.
Keep the installation identifier stable across launches, create a session ID
for the logical session, and create a boot ID for each process launch. The
installation identifier is used only as deterministic sampling input; do not
use an email address, account name, advertising identifier, or other direct
identity.

```swift
import Chill
import Foundation

@MainActor
enum Telemetry {
  static func configure(
    endpoint: URL,
    credential: String,
    installationID: String,
    allowInsecureLocalhost: Bool = false
  ) throws {
    let support = FileManager.default.urls(
      for: .applicationSupportDirectory,
      in: .userDomainMask
    )[0]
    let queue = support.appending(path: "Chill/Events", directoryHint: .isDirectory)

    let exporter = try ChillOfflinePipeline(
      directory: queue,
      configuration: ChillOTLPConfiguration(
        endpoint: endpoint,
        headers: ["authorization": "Bearer \(credential)"],
        resourceAttributes: [
          "service.name": "cats-ios",
          "deployment.environment.name": "internal",
        ],
        allowInsecureLocalhost: allowInsecureLocalhost
      )
    )

    let privacy = try PrivacyPolicy(
      version: "privacy-v1",
      analytics: .granted,
      diagnostic: .granted,
      replay: .denied,
      annotationAllowlist: [
        "account.tier": .internalData,
        "cat.id": .pseudonymousIdentifier,
      ]
    )
    let runtime = ChillRuntime(
      configuration: RuntimeConfiguration(
        sessionID: try SessionID(UUIDv7.generate().uuidString.lowercased()),
        bootID: try BootID(UUID().uuidString.lowercased()),
        privacy: privacy,
        sampling: try SamplingConfiguration(
          stableKey: installationID,
          salt: "cats-internal-v1",
          behavior: .all,
          replay: .none
        )
      ),
      sink: exporter
    )
    Chill.configure(runtime)
  }
}
```

Use HTTPS in every deployed environment. Plain HTTP is accepted only for an
explicit loopback endpoint when `allowInsecureLocalhost` is true. Credentials
stay in memory; do not put them in resource attributes, annotations, URLs, or
logs. The queue defaults to 64 MiB, persists before network delivery, recovers
after process death, and retries with bounded jitter.

Calling `flush()` is not normal instrumentation. The exporter drains on its
own. Use `await exporter.flush()` only for an explicit lifecycle drain or a
test, and `await exporter.shutdown()` only when the process truly stops owning
the pipeline.

## Declare pages, annotations, actions, and state events

Page names describe stable product structure. Dynamic IDs belong in bounded
annotations, never in page segments or event names.

```swift
import Chill
import SwiftUI

struct CatDetailScreen: View {
  let catID: String
  let catName: String
  let requestAdoption: @MainActor () -> Void
  @State private var isFavorite = false

  var body: some View {
    VStack(spacing: 24) {
      Image(systemName: "cat.fill")
        .font(.largeTitle)
        .impression("cat.hero", role: "image")
        .replay(.masked)

      Text(catName)
        .privacy(.redacted)

      ChillToggle("cat.favorite", isOn: $isFavorite) {
        Text("Favorite", comment: "Toggle that favorites the displayed cat.")
      }

      ChillActionButton("cat.adopt", operation: requestAdoption) {
        Text("Request adoption", comment: "Button that starts a cat adoption request.")
      }
    }
    .annotation("cat.id", value: catID)
    .event("cat.favorite.changed", when: isFavorite)
    .page("cat")
  }
}

struct RootScreen: View {
  let catID: String
  let catName: String
  let requestAdoption: @MainActor () -> Void

  var body: some View {
    NavigationStack {
      CatDetailScreen(
        catID: catID,
        catName: catName,
        requestAdoption: requestAdoption
      )
    }
    .annotation("account.tier", value: "internal")
    .page("app", relation: .root)
  }
}
```

This produces a structured `app / cat` page path. A sheet can declare
`.page("adoption", relation: .sheet, cause: .present)` and a tab can declare
`.page("settings", relation: .tab, cause: .selection)`. Chill assigns surface
and page-instance identity; the application does not construct path strings or
trace IDs.

Prefer `ChillActionButton`, `ChillToggle`, and `ChillPicker` for their native
commit boundaries. Use `.action(...)` when an existing view or submit boundary
cannot adopt a Chill control. `.event(..., when:)` observes an `Equatable`
state projection but never serializes the state value. `.impression(...)`
emits only after the declared visible-area and dwell thresholds are met and
never reads the visible label or pixels.

Keep declarations attached unconditionally. A conditional view-modifier helper
can replace the view's structural identity and reset state when its condition
changes. Use consent, sampling, remote collection state, and monotone privacy
restrictions to control capture instead.

### Outer-first annotation resolution

An ancestor's annotation is authoritative. If `RootScreen` declares
`account.tier = internal`, a descendant cannot replace it. Chill records the
collision and keeps the outer value. Sibling scopes remain isolated.

SwiftUI modifier syntax wraps inside-out, so later modifiers on the same view
are structurally outer:

```swift
Text("Cat")
  .annotation("account.tier", value: "detail")
  .annotation("account.tier", value: "internal") // outer; wins
```

Dictionary declarations are homogeneous and sorted deterministically:

```swift
ContentView()
  .annotation([
    "app.area": "adoption",
    "build.channel": "internal",
  ])
```

For shared keys, use a typed `AnnotationKey<Value>`. Only strings, booleans,
integers, finite doubles, and homogeneous arrays of those scalars are accepted.
Every emitted annotation must also appear in the privacy policy's allowlist
with an eligible classification; otherwise it is omitted before buffering.

## Instrument activities and domain events with macros

Macros preserve the original function signature, isolation, generics, async
behavior, typed errors, and return value. They instrument the body without an
imperative telemetry statement.

```swift
import Chill

struct Cat: Sendable {
  let id: String
}

struct CatRepository: Sendable {
  @Activity("repository.load_cat", kind: .network)
  func loadCat(id: String) async throws -> Cat {
    // Existing repository implementation.
    Cat(id: id)
  }

  @Event(
    "adoption.requested",
    eventClass: .domain,
    severity: .info,
    emission: .terminal,
    deduplicationKey: requestID
  )
  func requestAdoption(requestID: String) async throws {
    // Existing domain implementation.
  }
}
```

`@Activity` emits paired start/end records and creates a child trace span when
an OpenTelemetry bridge is configured. Use role `.attempt` only inside another
activity. `@Event` supports `entered`, `succeeded`, or `terminal`; observed
state belongs on `.event(..., when:)`. A deduplication key is hashed before it
enters a record.

Macro names must be static semantic literals. Invalid names are compile-time
diagnostics. View and UIKit declaration names are validated at runtime and fail
closed as no-ops when invalid.

## UIKit integration

UIKit uses the same vocabulary in ordinary view/controller setup. No global
method swizzling or replacement app delegate is required.

```swift
let root = UINavigationController(rootViewController: CatsViewController())
  .annotation("account.tier", value: "internal")
  .page("app", relation: .root)

final class CatViewController: UIViewController {
  let catID: String
  let adoptButton = ChillButton(type: .system)

  init(catID: String) {
    self.catID = catID
    super.init(nibName: nil, bundle: nil)
    annotation("cat.id", value: catID)
    page("cat")
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { nil }

  override func viewDidLoad() {
    super.viewDidLoad()
    adoptButton.action("cat.adopt")
  }
}
```

Native navigation, tab, split, presentation, scene, control, and gesture
boundaries emit automatically after declaration. Chill's contextual control
and reusable-view subclasses preserve task-local annotation/page context across
target dispatch and reuse. See the compile-checked
[UIKit sample](../sdk/swift/Examples/Sources/ChillUIKitSample/ChillUIKitSample.swift)
for complete layout and control wiring.

## OpenTelemetry and network propagation

Behavior records are exported as lossless OTLP `LogRecord` values under the
stable `chill.*` mapping. Resource attributes come from
`ChillOTLPConfiguration`; record-specific identity, page, annotations, and
privacy state remain record attributes. Behavior and replay sampling are
independent of the OpenTelemetry trace sampled bit.

Wrap an existing `URLSession` once and allowlist only exact first-party origins:

```swift
let propagation = try ChillNetworkPropagationPolicy(
  trustedOriginURLs: [URL(string: "https://api.example.com")!],
  baggageAllowlist: ["release.channel"],
  baggage: ["release.channel": "internal"]
)
let network = URLSession.shared.chill(propagation)

@Activity("repository.load_cat", kind: .network)
func load(_ request: URLRequest) async throws -> Data {
  try await network.data(for: request).0
}
```

The adapter injects W3C `traceparent`, optional bounded `tracestate`, and only
explicitly allowlisted baggage. Trust is scheme + host + normalized port and is
checked again after every redirect. URL paths, queries, headers, bodies, error
descriptions, annotations, and identity are never converted into propagation
metadata.

Chill is provider-neutral and will join, not replace, an application's existing
OpenTelemetry provider. In this repository the provider hook is currently an
SPI reserved for an official adapter package. Application code should not
import `TraceIntegration` or implement that SPI directly. Until the official
adapter ships, Chill still exports OTLP behavior logs and creates/propagates
valid local W3C trace context for trusted requests, but spans do not enter the
application's provider.

The complete mapping is defined in the
[OpenTelemetry contract](architecture/adr-0005-opentelemetry-mapping-and-propagation.md)
and the exact trust boundary in the
[network propagation decision](architecture/adr-0012-apple-networking-trace-propagation.md).

## Consent, collection policy, and replay

Local consent is the only source that can grant collection. Remote policy can
disable the SDK or narrow capture classes, never broaden them. Refresh the
authenticated state at launch and foreground transitions:

```swift
let client = try ChillCollectionPolicyClient(
  endpoint: URL(string: "https://telemetry.example.com/v1/chill/collection-state")!,
  headers: ["authorization": "Bearer <project credential>"]
)
let policies = ChillCollectionPolicySynchronizer(runtime: runtime, client: client)
try await policies.refresh()
```

Retain the synchronizer with application services. A stale remote revision is
ignored. A remote/local privacy-policy version mismatch fails closed. When the
user changes consent, build a new `PrivacyPolicy` and call
`runtime.updatePrivacyPolicy(...)`; capture gates change immediately.

Replay requires its own consent, sampling decision, package import, and storage
configuration:

Before configuring replay, deliberately set `replay: .granted` on the local
privacy policy and choose a nonzero replay sampling rate in
`SamplingConfiguration`. Sampling is fixed when the runtime is created; the
safe bootstrap above uses `.denied` and `.none` and therefore cannot capture
replay until a newly configured runtime reflects that product decision.

```swift
import ChillReplay

let replay = try ChillReplayConfiguration.standard(
  directory: applicationSupport.appending(path: "Chill/Replay"),
  keyIdentifier: "cats-internal"
)
try ChillSessionReplay.configure(replay)
```

Replay is structural: it records bounded geometry, viewport/scroll state, and
gestures, not screenshots. Text becomes a length bucket, secure input stays
masked, and pixel/web/media/custom drawing regions are opaque. An ancestor's
`.privacy(.blocked)`, `.privacy(.redacted)`, `.replay(.masked)`, or
`.replay(.blocked)` restriction cannot be relaxed by a descendant.

The current SDK seals encrypted, digest-bound chunks and exposes
`pendingChunks()`, `encryptedPayload(for:)`, and `acknowledge(chunkIDs:)` for a
service uploader targeting the backend's authenticated `/v1/chill/replay`
route. Configuration does not upload replay bytes by itself. Do not write an
uploader that decrypts chunks or treats the behavior-log chunk metadata as the
payload.

On consent withdrawal, update the runtime policy and call
`try await ChillSessionReplay.disable(purgePending: true)` when policy requires
pending local replay to be erased.

## Migration from imperative analytics

Migrate one semantic surface at a time and compare old/new aggregate counts
before removing the legacy SDK:

| Existing intent | Chill declaration |
| --- | --- |
| Screen-view call with a dynamic route | Stable `.page(...)` segments plus annotations for IDs |
| Tap/submit callback telemetry | `ChillActionButton`, native Chill controls, or `.action(...)` |
| Timed repository/domain operation | `@Activity` on the owning function |
| Domain-success or terminal event | `@Event` on the owning function |
| State-change event | `.event(..., when:)` on the owning view |
| Manually passed trace ID | Existing OTel context plus the Chill network adapter |
| Screenshot replay SDK | Structural replay with explicit privacy/replay boundaries |

Do not reproduce the old event payload inside annotations. Start with stable
product semantics and a short allowlist. Keep legacy and Chill delivery
separate during comparison so at-least-once retries are not mistaken for user
actions. The backend deduplicates Chill records by record ID.

## Debugging and troubleshooting

Exporter and replay metrics are safe to expose in an internal diagnostics UI:

```swift
let delivery = await exporter.metrics()
let replay = await ChillSessionReplay.metrics()
```

Metrics contain counts and queue state, not credentials or captured content.

| Symptom | Checks |
| --- | --- |
| No behavior records | Configure before mounting UI; verify `Chill.isEnabled`, local analytics consent, behavior sampling, and a matching active remote policy. |
| Annotation absent | Add it to `PrivacyPolicy.annotationAllowlist`; use an eligible classification and a supported scalar/array value. |
| Declaration silently emits nothing | Check runtime strings against semantic-name/page-segment limits. Macro literals instead produce compiler diagnostics. |
| Wrong annotation value | Find the structurally outer declaration. Outer-first is intentional; collision diagnostics identify winner and loser. |
| Unexpected page path | Put stable segments on every owning container and keep entity IDs in annotations. Set sheet/tab relations at their actual container boundary. |
| Action count too high | Use one native committed boundary. Do not attach both `ChillActionButton` and `.action(...)` to the same interaction. |
| Queue does not drain | Inspect `consecutiveFailures`, `exportFailures`, queued bytes, endpoint HTTPS, authentication, and backend readiness. Durable data remains for retry. |
| Trace headers absent | Use the wrapped session, an exact origin URL without a path/query, and inspect redirect destinations. Third-party redirects deliberately strip context. |
| Replay absent | Check separate replay consent and sampling, `ChillReplay` configuration, ancestor restrictions, Keychain access, and pending-chunk metrics. |
| Replay metadata exists but no playback | Upload the encrypted chunk through the replay service path; OTLP contains correlation metadata, not replay bytes. |

For local end-to-end verification, bootstrap the backend as documented in the
[backend guide](../backend/README.md), set `CHILL_OTLP_ENDPOINT` and
`CHILL_API_KEY`, opt into loopback HTTP only for local testing, and run the
compile-checked examples. From the repository root, the complete Apple gate is:

```sh
scripts/validate-swift-sdk.sh
```

Physical-device release budgets remain a separate required gate; simulator and
host results are presubmit evidence only.
