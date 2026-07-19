import ChillCore
import SwiftUI
import os

struct PageFact: Equatable, Sendable {
  let path: PagePath
  let annotations: AnnotationSnapshot
  let policy: ViewCapturePolicy
}

final class PageLifetime: @unchecked Sendable {
  private struct State {
    var started = false
    var ended = false
    var latest: PageFact?
    var finishHandler: (@Sendable () -> Void)?
  }

  private let state = OSAllocatedUnfairLock(initialState: State())
  private let createdAt = ContinuousClock.now

  func appear(_ fact: PageFact, initialCause: PageCause) {
    let operationAndCause = state.withLock {
      state -> (
        BehaviorOperation, PageCause
      )? in
      guard !state.ended else { return nil }
      let isStart = !state.started
      let changed = state.latest != fact
      state.started = true
      state.latest = fact
      if isStart { return (.start, initialCause) }
      return changed ? (.update, .foreground) : nil
    }
    guard let (operation, cause) = operationAndCause else { return }
    ChillUIInstrumentation.page(
      operation: operation,
      path: fact.path,
      cause: cause,
      annotations: fact.annotations,
      policy: fact.policy
    )
    if operation == .start,
      let name = try? SemanticName("ui.page.first_render")
    {
      ChillUIInstrumentation.observedEvent(
        name: name,
        eventClass: .performance,
        severity: .info,
        captureClass: .diagnostic,
        durationNano: createdAt.duration(to: .now).clampedNanoseconds,
        context: SwiftUIFactContext(
          annotations: fact.annotations,
          page: fact.path,
          policy: fact.policy
        )
      )
    }
  }

  func update(_ fact: PageFact, cause: PageCause) {
    let shouldUpdate = state.withLock { state in
      guard state.started, !state.ended, state.latest != fact else {
        return false
      }
      state.latest = fact
      return true
    }
    guard shouldUpdate else { return }
    ChillUIInstrumentation.page(
      operation: .update,
      path: fact.path,
      cause: cause,
      annotations: fact.annotations,
      policy: fact.policy
    )
  }

  func setFinishHandler(_ handler: @escaping @Sendable () -> Void) {
    let invokeImmediately = state.withLock { state in
      guard !state.ended else { return true }
      state.finishHandler = handler
      return false
    }
    if invokeImmediately { handler() }
  }

  func finish() {
    let result = state.withLock {
      state -> (PageFact?, (@Sendable () -> Void)?)? in
      guard !state.ended else { return nil }
      state.ended = true
      let handler = state.finishHandler
      state.finishHandler = nil
      return (state.started ? state.latest : nil, handler)
    }
    guard let result else { return }
    if let fact = result.0 {
      ChillUIInstrumentation.page(
        operation: .end,
        path: fact.path,
        cause: terminalCause(for: fact.path.relation),
        annotations: fact.annotations,
        policy: fact.policy
      )
    }
    result.1?()
  }

  deinit {
    finish()
  }

  private func terminalCause(for relation: PageRelation) -> PageCause {
    switch relation {
    case .root: .surfaceDestroyed
    case .sheet, .popover, .overlay, .cover: .dismiss
    case .tab, .split: .selection
    case .push: .back
    }
  }
}

private struct ChillPageModifier: ViewModifier {
  @Environment(\.chillAnnotationContext) private var annotations
  @Environment(\.chillCapturePolicy) private var policy
  @Environment(\.chillPageAncestry) private var parent
  @Environment(\.chillPageSurfaceCoordinator) private var inheritedCoordinator
  @Environment(\.scenePhase) private var scenePhase
  @State private var instanceID = newPageInstanceID()
  @State private var rootSurfaceID = newSurfaceID()
  @State private var lifetime = PageLifetime()
  @State private var rootCoordinator = PageSurfaceCoordinator()
  @State private var mounted = false
  @State private var observedScenePhase: ScenePhase?

  let segment: PageSegment?
  let relation: PageRelation
  let exposure: PageExposure
  let focused: Bool
  let cause: PageCause

  func body(content: Content) -> some View {
    let coordinator = inheritedCoordinator ?? rootCoordinator
    let declaredFact = makeFact(
      exposure: scenePhase == .active ? exposure : .retained,
      focused: scenePhase == .active && focused
    )
    let activeFact = declaredFact.map { fact in
      PageFact(
        path: coordinator.resolve(fact.path),
        annotations: fact.annotations,
        policy: fact.policy
      )
    }
    content
      .environment(\.chillPageSurfaceCoordinator, coordinator)
      .environment(
        \.chillPageAncestry,
        activeFact.map { SwiftUIPageAncestry(path: $0.path) } ?? parent
      )
      .onAppear { [coordinator] in
        guard let declaredFact else { return }
        mounted = true
        coordinator.register(
          path: declaredFact.path,
          parentInstanceID: parent?.path.instanceID,
          declaredExposure: declaredFact.path.exposure,
          declaredFocused: declaredFact.path.focused
        )
        let activeFact = PageFact(
          path: coordinator.resolve(declaredFact.path),
          annotations: declaredFact.annotations,
          policy: declaredFact.policy
        )
        lifetime.setFinishHandler { [weak coordinator] in
          Task { @MainActor in
            coordinator?.remove(instanceID)
          }
        }
        if parent == nil {
          AppleDiagnosticsBridge.installIfNeeded()
        }
        lifetime.appear(
          activeFact,
          initialCause: parent == nil && cause == .navigate ? .initial : cause
        )
        observeScene(activeFact)
      }
      .onDisappear {
        mounted = false
        coordinator.setMounted(false, for: instanceID)
        guard let declaredFact else { return }
        let retained = PageFact(
          path: coordinator.resolve(declaredFact.path),
          annotations: declaredFact.annotations,
          policy: declaredFact.policy
        )
        lifetime.update(
          retained,
          cause: scenePhase == .active ? .navigate : .background
        )
      }
      .onChange(of: declaredFact) { _, fact in
        guard mounted, let fact else { return }
        coordinator.register(
          path: fact.path,
          parentInstanceID: parent?.path.instanceID,
          declaredExposure: fact.path.exposure,
          declaredFocused: fact.path.focused
        )
      }
      .onChange(of: activeFact) { _, fact in
        guard let fact else { return }
        lifetime.update(
          fact,
          cause: scenePhase == .active ? .foreground : .background
        )
        observeScene(fact)
      }
  }

  private func makeFact(
    exposure: PageExposure,
    focused: Bool
  ) -> PageFact? {
    guard let segment else { return nil }
    let surfaceID = parent?.path.surfaceID ?? rootSurfaceID
    let instanceIDs = (parent?.path.instanceIDs ?? []) + [instanceID]
    let segments = (parent?.path.segments ?? []) + [segment]
    guard
      let path = try? PagePath(
        surfaceID: surfaceID,
        instanceID: instanceID,
        instanceIDs: instanceIDs,
        segments: segments,
        relation: parent == nil ? .root : relation,
        exposure: exposure,
        focused: focused && exposure == .foreground
      )
    else {
      return nil
    }
    return PageFact(
      path: path,
      annotations: annotations.snapshot,
      policy: policy
    )
  }

  private func observeScene(_ fact: PageFact) {
    guard parent == nil, observedScenePhase != scenePhase else { return }
    observedScenePhase = scenePhase
    let rawName =
      switch scenePhase {
      case .active: "app.scene.active"
      case .inactive: "app.scene.inactive"
      case .background: "app.scene.background"
      @unknown default: "app.scene.unknown"
      }
    guard let name = try? SemanticName(rawName) else { return }
    ChillUIInstrumentation.observedEvent(
      name: name,
      eventClass: .lifecycle,
      severity: .info,
      context: SwiftUIFactContext(
        annotations: fact.annotations,
        page: fact.path,
        policy: fact.policy
      )
    )
  }
}

extension View {
  /// Declares one stable semantic page segment. Nested declarations compose
  /// into a structured path; dynamic entity IDs belong in annotations.
  public func page(
    _ segment: String,
    relation: PageRelation = .push,
    exposure: PageExposure = .foreground,
    focused: Bool = true,
    cause: PageCause = .navigate
  ) -> some View {
    modifier(
      ChillPageModifier(
        segment: try? PageSegment(segment),
        relation: relation,
        exposure: exposure,
        focused: focused,
        cause: cause
      )
    )
  }
}
