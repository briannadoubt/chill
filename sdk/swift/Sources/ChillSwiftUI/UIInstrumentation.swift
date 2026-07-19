import ChillCore

struct SwiftUIFactContext: Equatable, Sendable {
  let annotations: AnnotationSnapshot
  let page: PagePath?
  let policy: ViewCapturePolicy
}

enum ChillUIInstrumentation {
  static func page(
    operation: BehaviorOperation,
    path: PagePath,
    cause: PageCause,
    annotations: AnnotationSnapshot,
    policy: ViewCapturePolicy
  ) {
    guard policy.privacy != .blocked,
      let runtime = Chill.currentRuntime(),
      let name = path.segments.last.flatMap({ try? SemanticName($0.rawValue) })
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
    context: SwiftUIFactContext
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
        subjectID: SubjectID(
          UUIDv7.generate().uuidString.lowercased()
        ),
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
    context: SwiftUIFactContext
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
        subjectID: SubjectID(
          UUIDv7.generate().uuidString.lowercased()
        ),
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

  static func observedEvent(
    name: SemanticName,
    eventClass: EventClass,
    severity: EventSeverity,
    captureClass: CaptureClass = .analytics,
    durationNano: UInt64? = nil,
    context: SwiftUIFactContext
  ) {
    guard context.policy.privacy != .blocked,
      let runtime = Chill.currentRuntime()
    else {
      return
    }
    try! runtime.record(captureClass: captureClass) {
      try BehaviorDraft(
        subjectID: SubjectID(
          UUIDv7.generate().uuidString.lowercased()
        ),
        kind: .event,
        operation: .instant,
        name: name,
        annotations: context.annotations,
        page: context.page,
        redactionState: context.policy.redactionState,
        payload: .event(
          EventPayload(
            eventClass: eventClass,
            severity: severity,
            emission: .observed
          )
        ),
        durationNano: durationNano
      )
    }
  }
}
