import ChillCore
import SwiftUI
import Testing
import os

@testable import ChillSwiftUI

private final class SwiftUICollectingSink: RecordSink {
  private let storage = OSAllocatedUnfairLock(
    initialState: [BehaviorRecord]()
  )

  func submit(_ record: BehaviorRecord) {
    storage.withLock { $0.append(record) }
  }

  var records: [BehaviorRecord] {
    storage.withLock { $0 }
  }
}

private struct RepresentativeView: View {
  @State private var enabled = false
  @State private var selection = 1

  var body: some View {
    VStack {
      ChillActionButton("Adopt", action: "cat.adopt") {}
      ChillToggle("Enabled", action: "cat.enabled", isOn: $enabled)
      ChillPicker(
        "cat.selection",
        selection: $selection,
        content: {
          Text("One").tag(1)
          Text("Two").tag(2)
        },
        label: {
          Text("Cat")
        }
      )
      Button("Save") {}
        .action("cat.save")
      Text("Cat")
        .impression("cat.hero", role: "heading")
    }
    .annotation("cat.id", value: "cat-42")
    .annotation(["account.tier": "internal"])
    .event("cat.selection.changed", when: selection)
    .page("cat")
    .privacy(.redacted)
    .replay(.masked)
  }
}

@Suite("SwiftUI declarative instrumentation", .serialized)
struct SwiftUIInstrumentationTests {
  @Test("Privacy and replay restrictions are monotone")
  func capturePolicyIsMonotone() {
    let blocked = ViewCapturePolicy.standard.restricting(
      privacy: .blocked,
      replay: .masked
    )
    let attemptedRelaxation = blocked.restricting(
      privacy: .standard,
      replay: .automatic
    )

    #expect(attemptedRelaxation.privacy == .blocked)
    #expect(attemptedRelaxation.replay == .masked)
  }

  @Test("Page lifetimes emit start, changed update, and exactly one end")
  func pageLifetime() throws {
    let sink = try configureSwiftUIRuntime()
    defer { Chill.disable() }
    let foreground = try pagePath(exposure: .foreground, focused: true)
    let retained = try pagePath(exposure: .retained, focused: false)
    let lifetime = PageLifetime()

    lifetime.appear(
      PageFact(
        path: foreground,
        annotations: .empty,
        policy: .standard
      ),
      initialCause: .navigate
    )
    lifetime.appear(
      PageFact(
        path: foreground,
        annotations: .empty,
        policy: .standard
      ),
      initialCause: .navigate
    )
    lifetime.update(
      PageFact(
        path: retained,
        annotations: .empty,
        policy: .standard
      ),
      cause: .background
    )
    lifetime.finish()
    lifetime.finish()

    let pages = sink.records.filter { $0.kind == .page }
    let performance = sink.records.filter {
      $0.name.rawValue == "ui.page.first_render"
    }
    #expect(pages.map(\.operation) == [.start, .update, .end])
    #expect(pages.map(\.subjectID).count == 3)
    #expect(Set(pages.map(\.subjectID)).count == 1)
    #expect(performance.count == 1)
    #expect(performance.first?.durationNano != nil)
    #expect(performance.first?.captureClass == .diagnostic)
  }

  @Test("One surface coordinates nested exposure and focus")
  @MainActor
  func pageSurfaceCoordination() throws {
    let coordinator = PageSurfaceCoordinator()
    let root = try pagePath(
      instanceIDs: ["root"],
      segments: ["app"],
      relation: .root
    )
    let pushed = try pagePath(
      instanceIDs: ["root", "pushed"],
      segments: ["app", "cat"],
      relation: .push
    )

    coordinator.register(
      path: root,
      parentInstanceID: nil,
      declaredExposure: .foreground,
      declaredFocused: true
    )
    coordinator.register(
      path: pushed,
      parentInstanceID: root.instanceID,
      declaredExposure: .foreground,
      declaredFocused: true
    )

    #expect(coordinator.resolve(root).exposure == .retained)
    #expect(coordinator.resolve(root).focused == false)
    #expect(coordinator.resolve(pushed).exposure == .foreground)
    #expect(coordinator.resolve(pushed).focused == true)

    coordinator.setMounted(false, for: pushed.instanceID)

    #expect(coordinator.resolve(root).exposure == .foreground)
    #expect(coordinator.resolve(root).focused == true)
    #expect(coordinator.resolve(pushed).exposure == .retained)
    #expect(coordinator.resolve(pushed).focused == false)
  }

  @Test("Covers occlude presenters and split panes choose one focus")
  @MainActor
  func presentationAndSplitCoordination() throws {
    let coordinator = PageSurfaceCoordinator()
    let root = try pagePath(
      instanceIDs: ["root"],
      segments: ["app"],
      relation: .root
    )
    let cover = try pagePath(
      instanceIDs: ["root", "cover"],
      segments: ["app", "paywall"],
      relation: .cover
    )
    coordinator.register(
      path: root,
      parentInstanceID: nil,
      declaredExposure: .foreground,
      declaredFocused: true
    )
    coordinator.register(
      path: cover,
      parentInstanceID: root.instanceID,
      declaredExposure: .foreground,
      declaredFocused: true
    )

    #expect(coordinator.resolve(root).exposure == .occluded)
    #expect(coordinator.resolve(cover).focused == true)

    coordinator.remove(cover.instanceID)
    let primary = try pagePath(
      instanceIDs: ["root", "primary"],
      segments: ["app", "primary"],
      relation: .split
    )
    let detail = try pagePath(
      instanceIDs: ["root", "detail"],
      segments: ["app", "detail"],
      relation: .split
    )
    coordinator.register(
      path: primary,
      parentInstanceID: root.instanceID,
      declaredExposure: .foreground,
      declaredFocused: true
    )
    coordinator.register(
      path: detail,
      parentInstanceID: root.instanceID,
      declaredExposure: .foreground,
      declaredFocused: true
    )

    #expect(coordinator.resolve(root).exposure == .visible)
    #expect(coordinator.resolve(primary).focused == false)
    #expect(coordinator.resolve(detail).focused == true)
  }

  @Test("UI facts snapshot context and obey a blocked subtree")
  func actionAndImpressionFacts() throws {
    let sink = try configureSwiftUIRuntime()
    defer { Chill.disable() }
    let path = try pagePath(exposure: .foreground, focused: true)
    let annotations = try AnnotationContext().addingScope(
      id: AnnotationScopeID("test-scope"),
      declarations: [
        AnnotationDeclaration("cat.id", value: "cat-42")
      ]
    ).snapshot
    let context = SwiftUIFactContext(
      annotations: annotations,
      page: path,
      policy: .standard
    )
    let name = try SemanticName("cat.adopt")
    let role = try SemanticName("button")
    let surfaceID = path.surfaceID
    let elementID = try ElementInstanceID("element-1")

    ChillUIInstrumentation.action(
      name: name,
      role: role,
      activation: .primary,
      input: .unknown,
      surfaceID: surfaceID,
      elementInstanceID: elementID,
      context: context
    )
    ChillUIInstrumentation.impression(
      name: name,
      role: role,
      visibilityRatio: 0.5,
      visibleDurationNano: 500_000_000,
      surfaceID: surfaceID,
      elementInstanceID: elementID,
      context: context
    )
    ChillUIInstrumentation.action(
      name: name,
      role: role,
      activation: .primary,
      input: .unknown,
      surfaceID: surfaceID,
      elementInstanceID: elementID,
      context: SwiftUIFactContext(
        annotations: annotations,
        page: path,
        policy: ViewCapturePolicy(privacy: .blocked)
      )
    )

    #expect(sink.records.map(\.kind) == [.action, .impression])
    #expect(sink.records.allSatisfy { $0.annotations.values == annotations.values })
    #expect(
      sink.records.allSatisfy {
        $0.annotations.classifications[try! AnnotationName("cat.id")]
          == .pseudonymousIdentifier
      }
    )
    #expect(sink.records.allSatisfy { $0.page == path })
    #expect(sink.records.allSatisfy { $0.element?.instanceID == elementID })
    guard case .action(let actionPayload) = sink.records.first?.payload,
      case .impression(let impressionPayload) = sink.records.last?.payload
    else {
      Issue.record("Expected typed action and impression payloads")
      return
    }
    #expect(actionPayload.elementID == name)
    #expect(impressionPayload.elementID == name)
    #expect(impressionPayload.visibilityRatio == 0.5)
  }

  @Test("Impression visibility uses clipped geometric area")
  func impressionVisibility() {
    let view = CGRect(x: 0, y: 0, width: 100, height: 80)

    #expect(ImpressionPolicy(visibilityThreshold: .nan).visibilityThreshold == 1)

    #expect(
      visibilityThresholdReached(
        viewBounds: view,
        viewportBounds: CGRect(x: 50, y: 0, width: 100, height: 80),
        threshold: 0.5
      )
    )
    #expect(
      !visibilityThresholdReached(
        viewBounds: view,
        viewportBounds: CGRect(x: 60, y: 0, width: 100, height: 80),
        threshold: 0.5
      )
    )
    #expect(
      !visibilityThresholdReached(
        viewBounds: .zero,
        viewportBounds: nil,
        threshold: 0
      )
    )
  }

  @Test("Native action context flows into generated activities")
  func actionContextPropagation() throws {
    let sink = try configureSwiftUIRuntime()
    defer { Chill.disable() }
    let path = try pagePath(exposure: .foreground, focused: true)
    let annotations = try AnnotationContext().addingScope(
      id: AnnotationScopeID("activity-context"),
      declarations: [
        AnnotationDeclaration("cat.id", value: "cat-42")
      ]
    ).snapshot

    _ChillInstrumentation.withUIContext(
      annotations: annotations,
      page: path
    ) {
      _ChillInstrumentation.withActivity(
        name: "cat.load",
        kind: .domain,
        role: .operation
      ) {}
    }

    #expect(sink.records.count == 2)
    #expect(sink.records.allSatisfy { $0.annotations.values == annotations.values })
    #expect(
      sink.records.allSatisfy {
        $0.annotations.classifications[try! AnnotationName("cat.id")]
          == .pseudonymousIdentifier
      }
    )
    #expect(sink.records.allSatisfy { $0.page == path })
  }

  @Test("Representative declarations compile without telemetry calls")
  @MainActor
  func representativeViewCompiles() {
    _ = RepresentativeView()
  }
}

private func configureSwiftUIRuntime() throws -> SwiftUICollectingSink {
  Chill.disable()
  let sink = SwiftUICollectingSink()
  Chill.configure(
    ChillRuntime(
      configuration: try RuntimeConfiguration(
        sessionID: SessionID("swiftui-session"),
        bootID: BootID("swiftui-boot"),
        privacy: PrivacyPolicy(
          version: "swiftui-privacy-v1",
          analytics: .granted,
          diagnostic: .granted,
          annotationAllowlist: [
            "account.tier": .internalData,
            "cat.id": .pseudonymousIdentifier,
          ]
        ),
        sampling: SamplingConfiguration(
          stableKey: "swiftui-user",
          salt: "swiftui-salt"
        )
      ),
      sink: sink
    )
  )
  return sink
}

private func pagePath(
  exposure: PageExposure,
  focused: Bool
) throws -> PagePath {
  let surfaceID = try SurfaceID("surface-1")
  let instanceID = try PageInstanceID("page-1")
  return try PagePath(
    surfaceID: surfaceID,
    instanceID: instanceID,
    instanceIDs: [instanceID],
    segments: [PageSegment("cat")],
    relation: .root,
    exposure: exposure,
    focused: focused
  )
}

private func pagePath(
  instanceIDs rawInstanceIDs: [String],
  segments rawSegments: [String],
  relation: PageRelation
) throws -> PagePath {
  let instanceIDs = try rawInstanceIDs.map(PageInstanceID.init)
  return try PagePath(
    surfaceID: SurfaceID("surface-1"),
    instanceID: instanceIDs.last!,
    instanceIDs: instanceIDs,
    segments: try rawSegments.map(PageSegment.init),
    relation: relation,
    exposure: .foreground,
    focused: true
  )
}
