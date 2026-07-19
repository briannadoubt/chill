public enum BehaviorKind: String, Codable, Sendable {
  case session
  case journey
  case page
  case impression
  case action
  case activity
  case event
  case replay
}

public enum BehaviorOperation: String, Codable, Sendable {
  case instant
  case start
  case update
  case end
}

public enum RedactionState: String, Codable, Sendable {
  case none
  case applied
  case blocked
}

public enum ActionActivation: String, Codable, Sendable {
  case primary
  case submit
  case toggle
  case selection
  case adjust
  case gesture
  case system
}

public enum InputKind: String, Codable, Sendable {
  case touch
  case pointer
  case keyboard
  case remote
  case accessibility
  case voice
  case system
  case unknown
}

public enum PageCause: String, Codable, Sendable {
  case initial
  case navigate
  case back
  case selection
  case present
  case dismiss
  case replace
  case deepLink = "deep_link"
  case restore
  case adaptive
  case background
  case foreground
  case surfaceDestroyed = "surface_destroyed"
}

public struct PagePayload: Equatable, Sendable {
  public let path: PagePath
  public let cause: PageCause
  public let navigationID: SubjectID?
  public let visibilityRatio: Double?

  public init(
    path: PagePath,
    cause: PageCause,
    navigationID: SubjectID? = nil,
    visibilityRatio: Double? = nil
  ) throws {
    if let visibilityRatio {
      guard visibilityRatio.isFinite,
        (0...1).contains(visibilityRatio)
      else {
        throw ContractError.invalidVisibilityRatio
      }
    }
    self.path = path
    self.cause = cause
    self.navigationID = navigationID
    self.visibilityRatio = visibilityRatio
  }
}

public struct ImpressionPayload: Equatable, Sendable {
  public let elementID: SemanticName
  public let role: SemanticName
  public let visibilityRatio: Double
  public let visibleDurationNano: UInt64?

  public init(
    elementID: SemanticName,
    role: SemanticName,
    visibilityRatio: Double,
    visibleDurationNano: UInt64? = nil
  ) throws {
    guard visibilityRatio.isFinite,
      (0...1).contains(visibilityRatio)
    else {
      throw ContractError.invalidVisibilityRatio
    }
    self.elementID = elementID
    self.role = role
    self.visibilityRatio = visibilityRatio
    self.visibleDurationNano = visibleDurationNano
  }
}

public struct ActionPayload: Equatable, Sendable {
  public let elementID: SemanticName
  public let role: SemanticName
  public let activation: ActionActivation
  public let input: InputKind

  public init(
    elementID: SemanticName,
    role: SemanticName,
    activation: ActionActivation,
    input: InputKind
  ) {
    self.elementID = elementID
    self.role = role
    self.activation = activation
    self.input = input
  }
}

public enum ActivityKind: String, Codable, Sendable {
  case ui
  case domain
  case network
  case storage
  case task
  case custom
}

public enum ActivityRole: String, Codable, Sendable {
  case operation
  case attempt
}

public struct ActivityPayload: Equatable, Sendable {
  public let kind: ActivityKind
  public let role: ActivityRole
  public let parentActivityID: SubjectID?
  public let attempt: UInt32
  public let recursionDepth: UInt32
  public let outcome: TerminalOutcome?
  public let durationNano: UInt64?
  public let reasonCode: SemanticName?

  public init(
    kind: ActivityKind,
    role: ActivityRole,
    parentActivityID: SubjectID? = nil,
    attempt: UInt32 = 1,
    recursionDepth: UInt32 = 0,
    outcome: TerminalOutcome? = nil,
    durationNano: UInt64? = nil,
    reasonCode: SemanticName? = nil
  ) {
    self.kind = kind
    self.role = role
    self.parentActivityID = parentActivityID
    self.attempt = attempt
    self.recursionDepth = recursionDepth
    self.outcome = outcome
    self.durationNano = durationNano
    self.reasonCode = reasonCode
  }
}

public enum EventClass: String, Codable, Sendable {
  case lifecycle
  case domain
  case error
  case crash
  case performance
  case experiment
  case custom
}

public enum EventSeverity: String, Codable, Sendable {
  case trace
  case debug
  case info
  case warn
  case error
  case fatal
}

public enum EventEmission: String, Codable, Sendable {
  case entered
  case succeeded
  case terminal
  case observed
}

public struct EventPayload: Equatable, Sendable {
  public let eventClass: EventClass
  public let severity: EventSeverity
  public let emission: EventEmission
  public let outcome: TerminalOutcome?
  public let deduplicationKeyHash: String?

  public init(
    eventClass: EventClass,
    severity: EventSeverity,
    emission: EventEmission,
    outcome: TerminalOutcome? = nil,
    deduplicationKeyHash: String? = nil
  ) {
    self.eventClass = eventClass
    self.severity = severity
    self.emission = emission
    self.outcome = outcome
    self.deduplicationKeyHash = deduplicationKeyHash
  }
}

public struct ReplayPayload: Equatable, Sendable {
  public let replayID: String
  public let chunkID: String
  public let chunkIndex: UInt32
  public let startsAtUnixNano: UInt64
  public let endsAtUnixNano: UInt64
  public let sha256: String
  public let byteCount: UInt64
  public let storageRef: String

  public init(
    replayID: String,
    chunkID: String,
    chunkIndex: UInt32,
    startsAtUnixNano: UInt64,
    endsAtUnixNano: UInt64,
    sha256: String,
    byteCount: UInt64,
    storageRef: String
  ) throws {
    self.replayID = try _validatedIdentifier(replayID, type: "replay ID")
    self.chunkID = try _validatedIdentifier(chunkID, type: "replay chunk ID")
    self.chunkIndex = chunkIndex
    self.startsAtUnixNano = startsAtUnixNano
    guard endsAtUnixNano >= startsAtUnixNano else {
      throw ContractError.invalidReplayTimeRange
    }
    self.endsAtUnixNano = endsAtUnixNano
    self.sha256 = try _validatedIdentifier(sha256, type: "replay digest")
    self.byteCount = byteCount
    self.storageRef = try _validatedIdentifier(
      storageRef,
      type: "replay storage reference"
    )
  }
}

public enum BehaviorPayload: Equatable, Sendable {
  case none
  case page(PagePayload)
  case impression(ImpressionPayload)
  case action(ActionPayload)
  case activity(ActivityPayload)
  case event(EventPayload)
  case replay(ReplayPayload)
}
