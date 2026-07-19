#if os(iOS)
  import ChillCore
  import Foundation
  import os

  struct UIKitFactContext: Equatable, Sendable {
    let annotations: AnnotationSnapshot
    let page: PagePath?
    let policy: ViewCapturePolicy
  }

  enum UIKitFactEmitter {
    static func page(
      operation: BehaviorOperation,
      path: PagePath,
      cause: PageCause,
      annotations: AnnotationSnapshot,
      policy: ViewCapturePolicy
    ) {
      guard policy.privacy != .blocked,
        let runtime = Chill.currentRuntime(),
        let name = path.segments.last.flatMap({
          try? SemanticName($0.rawValue)
        })
      else {
        return
      }
      try! runtime.record(captureClass: .analytics) {
        try BehaviorDraft(
          subjectID: SubjectID(path.instanceID.rawValue),
          kind: .page,
          operation: operation,
          name: name,
          annotations: annotations,
          page: path,
          redactionState: policy.redactionState,
          payload: .page(
            try PagePayload(
              path: path,
              cause: cause,
              visibilityRatio: path.exposure == .foreground ? 1 : nil
            )
          )
        )
      }
    }

    static func action(
      name: SemanticName,
      role: SemanticName,
      activation: ActionActivation,
      input: InputKind,
      surfaceID: SurfaceID,
      elementInstanceID: ElementInstanceID,
      context: UIKitFactContext
    ) {
      guard context.policy.privacy != .blocked,
        let runtime = Chill.currentRuntime()
      else {
        return
      }
      let element = ElementIdentity(
        surfaceID: surfaceID,
        instanceID: elementInstanceID,
        name: name,
        role: role
      )
      try! runtime.record(captureClass: .analytics) {
        try BehaviorDraft(
          subjectID: SubjectID(UUIDv7.generate().uuidString.lowercased()),
          kind: .action,
          operation: .instant,
          name: name,
          annotations: context.annotations,
          page: context.page,
          element: element,
          redactionState: context.policy.redactionState,
          payload: .action(
            ActionPayload(
              elementID: name,
              role: role,
              activation: activation,
              input: input
            )
          )
        )
      }
    }

    static func impression(
      name: SemanticName,
      role: SemanticName,
      visibilityRatio: Double,
      visibleDurationNano: UInt64,
      surfaceID: SurfaceID,
      elementInstanceID: ElementInstanceID,
      context: UIKitFactContext
    ) {
      guard context.policy.privacy != .blocked,
        let runtime = Chill.currentRuntime()
      else {
        return
      }
      let element = ElementIdentity(
        surfaceID: surfaceID,
        instanceID: elementInstanceID,
        name: name,
        role: role
      )
      try! runtime.record(captureClass: .analytics) {
        try BehaviorDraft(
          subjectID: SubjectID(UUIDv7.generate().uuidString.lowercased()),
          kind: .impression,
          operation: .instant,
          name: name,
          annotations: context.annotations,
          page: context.page,
          element: element,
          redactionState: context.policy.redactionState,
          payload: .impression(
            try ImpressionPayload(
              elementID: name,
              role: role,
              visibilityRatio: visibilityRatio,
              visibleDurationNano: visibleDurationNano
            )
          )
        )
      }
    }

    static func lifecycleEvent(
      name: SemanticName,
      context: UIKitFactContext
    ) {
      guard context.policy.privacy != .blocked,
        let runtime = Chill.currentRuntime()
      else {
        return
      }
      try! runtime.record(captureClass: .analytics) {
        try BehaviorDraft(
          subjectID: SubjectID(UUIDv7.generate().uuidString.lowercased()),
          kind: .event,
          operation: .instant,
          name: name,
          annotations: context.annotations,
          page: context.page,
          redactionState: context.policy.redactionState,
          payload: .event(
            EventPayload(
              eventClass: .lifecycle,
              severity: .info,
              emission: .observed
            )
          )
        )
      }
    }

    static func duplicateActionObservation(
      context: UIKitFactContext
    ) {
      guard context.policy.privacy != .blocked,
        let runtime = Chill.currentRuntime(),
        let name = try? SemanticName("action.duplicate_observation")
      else {
        return
      }
      try! runtime.record(captureClass: .diagnostic) {
        try BehaviorDraft(
          subjectID: SubjectID(UUIDv7.generate().uuidString.lowercased()),
          kind: .event,
          operation: .instant,
          name: name,
          annotations: context.annotations,
          page: context.page,
          redactionState: context.policy.redactionState,
          payload: .event(
            EventPayload(
              eventClass: .custom,
              severity: .debug,
              emission: .observed
            )
          )
        )
      }
    }

    static func firstRender(
      durationNano: UInt64,
      context: UIKitFactContext
    ) {
      guard context.policy.privacy != .blocked,
        let runtime = Chill.currentRuntime(),
        let name = try? SemanticName("ui.page.first_render")
      else {
        return
      }
      try! runtime.record(captureClass: .diagnostic) {
        try BehaviorDraft(
          subjectID: SubjectID(UUIDv7.generate().uuidString.lowercased()),
          kind: .event,
          operation: .instant,
          name: name,
          annotations: context.annotations,
          page: context.page,
          redactionState: context.policy.redactionState,
          payload: .event(
            EventPayload(
              eventClass: .performance,
              severity: .info,
              emission: .observed
            )
          ),
          durationNano: durationNano
        )
      }
    }
  }

  struct UIKitPageFact: Equatable, Sendable {
    let path: PagePath
    let annotations: AnnotationSnapshot
    let policy: ViewCapturePolicy
  }

  final class UIKitPageLifetime: @unchecked Sendable {
    private struct State {
      var started = false
      var ended = false
      var latest: UIKitPageFact?
    }

    private let state = OSAllocatedUnfairLock(initialState: State())
    private let createdAt = ContinuousClock.now

    var hasEnded: Bool {
      state.withLock(\.ended)
    }

    var latest: UIKitPageFact? {
      state.withLock(\.latest)
    }

    func appear(_ fact: UIKitPageFact, initialCause: PageCause) {
      let operationAndCause = state.withLock {
        state -> (BehaviorOperation, PageCause)? in
        guard !state.ended else { return nil }
        let isStart = !state.started
        let changed = state.latest != fact
        state.started = true
        state.latest = fact
        if isStart { return (.start, initialCause) }
        return changed ? (.update, .foreground) : nil
      }
      guard let (operation, cause) = operationAndCause else { return }
      UIKitFactEmitter.page(
        operation: operation,
        path: fact.path,
        cause: cause,
        annotations: fact.annotations,
        policy: fact.policy
      )
      if operation == .start {
        UIKitFactEmitter.firstRender(
          durationNano: createdAt.duration(to: .now).uiKitNanoseconds,
          context: UIKitFactContext(
            annotations: fact.annotations,
            page: fact.path,
            policy: fact.policy
          )
        )
      }
    }

    func update(_ fact: UIKitPageFact, cause: PageCause) {
      let shouldUpdate = state.withLock { state in
        guard state.started, !state.ended, state.latest != fact else {
          return false
        }
        state.latest = fact
        return true
      }
      guard shouldUpdate else { return }
      UIKitFactEmitter.page(
        operation: .update,
        path: fact.path,
        cause: cause,
        annotations: fact.annotations,
        policy: fact.policy
      )
    }

    func finish() {
      let fact = state.withLock { state -> UIKitPageFact? in
        guard !state.ended else { return nil }
        state.ended = true
        return state.started ? state.latest : nil
      }
      guard let fact else { return }
      UIKitFactEmitter.page(
        operation: .end,
        path: fact.path,
        cause: terminalCause(for: fact.path.relation),
        annotations: fact.annotations,
        policy: fact.policy
      )
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

  @MainActor
  final class UIKitPageSurfaceGraph {
    private struct Entry: Equatable {
      let parentInstanceID: PageInstanceID?
      let relation: PageRelation
      let declaredExposure: PageExposure
      let declaredFocused: Bool
      let order: UInt64
    }

    private var entries: [PageInstanceID: Entry] = [:]
    private var nextOrder: UInt64 = 0

    func register(
      path: PagePath,
      parentInstanceID: PageInstanceID?,
      declaredExposure: PageExposure,
      declaredFocused: Bool
    ) {
      let existing = entries[path.instanceID]
      if existing == nil { nextOrder &+= 1 }
      entries[path.instanceID] = Entry(
        parentInstanceID: parentInstanceID,
        relation: path.relation,
        declaredExposure: declaredExposure,
        declaredFocused: declaredFocused,
        order: existing?.order ?? nextOrder
      )
    }

    func remove(_ instanceID: PageInstanceID) {
      entries.removeValue(forKey: instanceID)
    }

    func resolve(_ path: PagePath) -> PagePath {
      guard entries[path.instanceID] != nil else { return path }
      let exposure = effectiveExposure(for: path.instanceID)
      let focused =
        exposure == .foreground
        && focusedInstanceID == path.instanceID
      return
        (try? PagePath(
          surfaceID: path.surfaceID,
          instanceID: path.instanceID,
          instanceIDs: path.instanceIDs,
          segments: path.segments,
          relation: path.relation,
          exposure: exposure,
          focused: focused
        )) ?? path
    }

    private var focusedInstanceID: PageInstanceID? {
      var candidates: [(PageInstanceID, Int, UInt64)] = []
      for (instanceID, entry) in entries {
        guard entry.declaredFocused,
          effectiveExposure(for: instanceID) == .foreground
        else {
          continue
        }
        candidates.append((instanceID, depth(of: instanceID), entry.order))
      }
      return candidates.max { left, right in
        if left.1 != right.1 { return left.1 < right.1 }
        return left.2 < right.2
      }?.0
    }

    private func effectiveExposure(
      for instanceID: PageInstanceID
    ) -> PageExposure {
      guard let entry = entries[instanceID] else { return .retained }
      let activeChildRelations = entries.compactMap { childID, child in
        child.parentInstanceID == instanceID && branchIsActive(at: childID)
          ? child.relation : nil
      }
      if activeChildRelations.contains(.cover) { return .occluded }
      if activeChildRelations.contains(where: {
        $0 == .push || $0 == .tab
      }) {
        return .retained
      }
      if activeChildRelations.contains(where: {
        $0 == .sheet || $0 == .popover || $0 == .overlay || $0 == .split
      }) {
        return .visible
      }
      return entry.declaredExposure
    }

    private func branchIsActive(at instanceID: PageInstanceID) -> Bool {
      guard let entry = entries[instanceID] else { return false }
      if entry.declaredExposure != .retained { return true }
      return entries.contains { childID, child in
        child.parentInstanceID == instanceID && branchIsActive(at: childID)
      }
    }

    private func depth(of instanceID: PageInstanceID) -> Int {
      var result = 0
      var cursor = entries[instanceID]?.parentInstanceID
      var visited: Set<PageInstanceID> = [instanceID]
      while let current = cursor, visited.insert(current).inserted {
        result += 1
        cursor = entries[current]?.parentInstanceID
      }
      return result
    }
  }

  func newUIKitAnnotationScopeID() -> AnnotationScopeID {
    try! AnnotationScopeID(UUIDv7.generate().uuidString.lowercased())
  }

  func newUIKitPageInstanceID() -> PageInstanceID {
    try! PageInstanceID(UUIDv7.generate().uuidString.lowercased())
  }

  func newUIKitElementInstanceID() -> ElementInstanceID {
    try! ElementInstanceID(UUIDv7.generate().uuidString.lowercased())
  }

  func newUIKitSurfaceID() -> SurfaceID {
    try! SurfaceID(UUIDv7.generate().uuidString.lowercased())
  }

  extension Duration {
    fileprivate var uiKitNanoseconds: UInt64 {
      let components = self.components
      guard components.seconds >= 0 else { return 0 }
      let seconds = UInt64(clamping: components.seconds)
      let attoseconds = max(components.attoseconds, 0)
      let nanos = UInt64(attoseconds / 1_000_000_000)
      let (whole, overflow) = seconds.multipliedReportingOverflow(
        by: 1_000_000_000
      )
      guard !overflow else { return .max }
      let (sum, additionOverflow) = whole.addingReportingOverflow(nanos)
      return additionOverflow ? .max : sum
    }
  }
#endif
