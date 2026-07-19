public enum TerminalOutcome: String, Codable, Sendable {
  case succeeded
  case failed
  case cancelled
  case timedOut = "timed_out"
}

public enum LifecyclePhase: Equatable, Sendable {
  case idle
  case active(startedMonotonicNano: UInt64)
  case terminal(outcome: TerminalOutcome, durationNano: UInt64)
}

public enum LifecycleDiagnostic: String, Equatable, Sendable {
  case duplicateStart = "lifecycle.duplicate_start"
  case unknownTerminal = "lifecycle.unknown_terminal"
  case duplicateTerminal = "lifecycle.duplicate_terminal"
  case monotonicTimeMovedBackwards = "lifecycle.monotonic_time_moved_backwards"
}

public enum LifecycleTransition: Equatable, Sendable {
  case started
  case ended(outcome: TerminalOutcome, durationNano: UInt64)
  case ignored(LifecycleDiagnostic)
}

public struct LifecycleState: Equatable, Sendable {
  public private(set) var phase: LifecyclePhase = .idle

  public init() {}

  @discardableResult
  public mutating func begin(at monotonicNano: UInt64) -> LifecycleTransition {
    guard case .idle = phase else {
      return .ignored(.duplicateStart)
    }
    phase = .active(startedMonotonicNano: monotonicNano)
    return .started
  }

  @discardableResult
  public mutating func finish(
    at monotonicNano: UInt64,
    outcome: TerminalOutcome
  ) -> LifecycleTransition {
    switch phase {
    case .idle:
      return .ignored(.unknownTerminal)
    case .terminal:
      return .ignored(.duplicateTerminal)
    case .active(let started):
      guard monotonicNano >= started else {
        return .ignored(.monotonicTimeMovedBackwards)
      }
      let duration = monotonicNano - started
      phase = .terminal(outcome: outcome, durationNano: duration)
      return .ended(outcome: outcome, durationNano: duration)
    }
  }
}
