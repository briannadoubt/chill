import CryptoKit
import Dispatch
import Foundation
import os

private final class ActivityFrame: Sendable {
  let subjectID: SubjectID
  let name: SemanticName
  let parent: ActivityFrame?
  let recursionDepth: UInt32
  private let attempts = OSAllocatedUnfairLock(
    initialState: [SemanticName: UInt32]()
  )

  init(
    subjectID: SubjectID,
    name: SemanticName,
    parent: ActivityFrame?,
    recursionDepth: UInt32
  ) {
    self.subjectID = subjectID
    self.name = name
    self.parent = parent
    self.recursionDepth = recursionDepth
  }

  func nextAttempt(for name: SemanticName) -> UInt32 {
    attempts.withLock { values in
      let next = values[name, default: 0] &+ 1
      values[name] = next
      return next
    }
  }
}

private enum ActivityTaskContext {
  @TaskLocal static var current: ActivityFrame?
}

private struct SemanticTaskSnapshot: Sendable {
  static let empty = SemanticTaskSnapshot(
    annotations: .empty,
    page: nil
  )

  let annotations: AnnotationSnapshot
  let page: PagePath?
}

private enum SemanticTaskContext {
  @TaskLocal static var current = SemanticTaskSnapshot.empty
}

private enum ActionTraceTaskContext {
  @TaskLocal static var isActive = false
}

private final class ActivityToken: Sendable {
  let runtime: ChillRuntime?
  let frame: ActivityFrame
  let kind: ActivityKind
  let role: ActivityRole
  let attempt: UInt32
  let started: TimePoint?
  let traceStartedMonotonicNano: UInt64
  let semanticContext: SemanticTaskSnapshot
  let traceSpan: ChillTraceSpan
  private let finished = OSAllocatedUnfairLock(initialState: false)

  init(
    runtime: ChillRuntime?,
    frame: ActivityFrame,
    kind: ActivityKind,
    role: ActivityRole,
    attempt: UInt32,
    started: TimePoint?,
    traceStartedMonotonicNano: UInt64,
    semanticContext: SemanticTaskSnapshot,
    traceSpan: ChillTraceSpan
  ) {
    self.runtime = runtime
    self.frame = frame
    self.kind = kind
    self.role = role
    self.attempt = attempt
    self.started = started
    self.traceStartedMonotonicNano = traceStartedMonotonicNano
    self.semanticContext = semanticContext
    self.traceSpan = traceSpan
  }

  func finish(_ outcome: TerminalOutcome) {
    let shouldFinish = finished.withLock { finished in
      guard !finished else { return false }
      finished = true
      return true
    }
    guard shouldFinish else { return }
    var behaviorDuration: UInt64?
    if let runtime, let started {
      let ended = runtime.timePoint()
      let duration =
        ended.monotonicNano >= started.monotonicNano
        ? ended.monotonicNano - started.monotonicNano
        : 0
      behaviorDuration = duration
      try! runtime.record(captureClass: .analytics, at: ended) {
        try BehaviorDraft(
          subjectID: frame.subjectID,
          kind: .activity,
          operation: .end,
          name: frame.name,
          annotations: semanticContext.annotations,
          page: semanticContext.page,
          trace: traceSpan.context,
          payload: .activity(
            ActivityPayload(
              kind: kind,
              role: role,
              parentActivityID: frame.parent?.subjectID,
              attempt: attempt,
              recursionDepth: frame.recursionDepth,
              outcome: outcome,
              durationNano: duration
            )
          ),
          durationNano: duration
        )
      }
    }
    let traceEndedMonotonicNano = DispatchTime.now().uptimeNanoseconds
    let traceDuration =
      behaviorDuration
      ?? (traceEndedMonotonicNano >= traceStartedMonotonicNano
        ? traceEndedMonotonicNano - traceStartedMonotonicNano
        : 0)
    let spanStatus: ChillTraceSpanStatus =
      switch outcome {
      case .succeeded: .ok
      case .cancelled: .unset
      case .failed, .timedOut: .error
      }
    traceSpan.end(
      status: spanStatus,
      attributes: [
        "chill.outcome": .string(outcome.rawValue),
        "chill.duration.nano": .integer(Int64(clamping: traceDuration)),
      ]
    )
  }
}

private struct EventToken: Sendable {
  let runtime: ChillRuntime
  let name: SemanticName
  let eventClass: EventClass
  let severity: EventSeverity
  let emission: EventEmission
  let deduplicationKey: String?
  let semanticContext: SemanticTaskSnapshot

  func emit(outcome: TerminalOutcome?) {
    let deduplicationKeyHash = deduplicationKey.map { key in
      SHA256.hash(data: Data(key.utf8)).map {
        String(format: "%02x", $0)
      }.joined()
    }
    guard runtime.claimEvent(name: name, keyHash: deduplicationKeyHash) else {
      return
    }
    let subjectID = try! SubjectID(
      UUIDv7.generate().uuidString.lowercased()
    )
    try! runtime.record(captureClass: .analytics) {
      try BehaviorDraft(
        subjectID: subjectID,
        kind: .event,
        operation: .instant,
        name: name,
        annotations: semanticContext.annotations,
        page: semanticContext.page,
        payload: .event(
          EventPayload(
            eventClass: eventClass,
            severity: severity,
            emission: emission,
            outcome: outcome,
            deduplicationKeyHash: deduplicationKeyHash
          )
        )
      )
    }
  }
}

@_documentation(visibility: internal)
public enum _ChillInstrumentation {
  package static func withUIContext<Result>(
    annotations: AnnotationSnapshot,
    page: PagePath?,
    operation: () -> Result
  ) -> Result {
    SemanticTaskContext.$current.withValue(
      SemanticTaskSnapshot(annotations: annotations, page: page),
      operation: operation
    )
  }

  /// Establishes one short internal dispatch span around a native semantic
  /// action. Structured child work inherits the context automatically.
  package static func withActionTrace<Result>(
    name: SemanticName?,
    operation: () -> Result
  ) -> Result {
    if ActionTraceTaskContext.isActive { return operation() }
    guard let name,
      Chill.currentRuntime()?.isCaptureEnabled(for: .analytics) == true
        || ChillTraceRuntime.isProviderConfigured
    else { return operation() }
    let span = ChillTraceRuntime.startSpan(
      name: name.rawValue,
      kind: .internal,
      attributes: ["chill.action.name": .string(name.rawValue)]
    )
    return TraceTaskContext.$current.withValue(span.context) {
      ActionTraceTaskContext.$isActive.withValue(true) {
        let result = operation()
        span.end(status: .ok)
        return result
      }
    }
  }

  public static func withActivity<Output, Failure: Error>(
    name rawName: String,
    kind: ActivityKind,
    role: ActivityRole,
    operation: () throws(Failure) -> Output
  ) throws(Failure) -> Output {
    guard let token = beginActivity(name: rawName, kind: kind, role: role) else {
      return try operation()
    }
    let execution: Result<Output, Failure> =
      TraceTaskContext.$current.withValue(token.traceSpan.context) {
        ActivityTaskContext.$current.withValue(token.frame) {
          do {
            let result = try operation()
            token.finish(.succeeded)
            return .success(result)
          } catch {
            token.finish(error is CancellationError ? .cancelled : .failed)
            // Swift's TaskLocal rethrows surface currently erases typed throws.
            // `operation` is the only throwing expression, so this cast is exact.
            return .failure(error as! Failure)
          }
        }
      }
    switch execution {
    case .success(let result): return result
    case .failure(let error): throw error
    }
  }

  public nonisolated(nonsending) static func withAsyncActivity<
    Output,
    Failure: Error
  >(
    name rawName: String,
    kind: ActivityKind,
    role: ActivityRole,
    operation: () async throws(Failure) -> Output
  ) async throws(Failure) -> Output {
    guard let token = beginActivity(name: rawName, kind: kind, role: role) else {
      return try await operation()
    }
    let execution: Result<Output, Failure> =
      await TraceTaskContext.$current.withValue(token.traceSpan.context) {
        await ActivityTaskContext.$current.withValue(token.frame) {
          do {
            let result = try await operation()
            token.finish(.succeeded)
            return .success(result)
          } catch {
            token.finish(error is CancellationError ? .cancelled : .failed)
            return .failure(error as! Failure)
          }
        }
      }
    switch execution {
    case .success(let result): return result
    case .failure(let error): throw error
    }
  }

  public static func withEvent<Result, Failure: Error>(
    name rawName: String,
    eventClass: EventClass,
    severity: EventSeverity,
    emission: EventEmission,
    deduplicationKey: String?,
    operation: () throws(Failure) -> Result
  ) throws(Failure) -> Result {
    guard
      let token = eventToken(
        name: rawName,
        eventClass: eventClass,
        severity: severity,
        emission: emission,
        deduplicationKey: deduplicationKey
      )
    else {
      return try operation()
    }
    if emission == .entered { token.emit(outcome: nil) }
    do {
      let result = try operation()
      if emission == .succeeded || emission == .terminal {
        token.emit(outcome: .succeeded)
      }
      return result
    } catch {
      if emission == .terminal {
        token.emit(
          outcome: error is CancellationError ? .cancelled : .failed
        )
      }
      throw error
    }
  }

  public nonisolated(nonsending) static func withAsyncEvent<
    Result,
    Failure: Error
  >(
    name rawName: String,
    eventClass: EventClass,
    severity: EventSeverity,
    emission: EventEmission,
    deduplicationKey: String?,
    operation: () async throws(Failure) -> Result
  ) async throws(Failure) -> Result {
    guard
      let token = eventToken(
        name: rawName,
        eventClass: eventClass,
        severity: severity,
        emission: emission,
        deduplicationKey: deduplicationKey
      )
    else {
      return try await operation()
    }
    if emission == .entered { token.emit(outcome: nil) }
    do {
      let result = try await operation()
      if emission == .succeeded || emission == .terminal {
        token.emit(outcome: .succeeded)
      }
      return result
    } catch {
      if emission == .terminal {
        token.emit(
          outcome: error is CancellationError ? .cancelled : .failed
        )
      }
      throw error
    }
  }

  private static func beginActivity(
    name rawName: String,
    kind: ActivityKind,
    role: ActivityRole
  ) -> ActivityToken? {
    guard let name = try? SemanticName(rawName) else { return nil }
    let runtime = Chill.currentRuntime().flatMap { runtime in
      runtime.isCaptureEnabled(for: .analytics) ? runtime : nil
    }
    guard runtime != nil || ChillTraceRuntime.isProviderConfigured else {
      return nil
    }
    let parent = ActivityTaskContext.current
    guard role != .attempt || parent != nil else { return nil }
    var recursionDepth: UInt32 = 0
    var ancestor = parent
    while let frame = ancestor {
      if frame.name == name { recursionDepth &+= 1 }
      ancestor = frame.parent
    }
    let subjectID = try! SubjectID(
      UUIDv7.generate().uuidString.lowercased()
    )
    let frame = ActivityFrame(
      subjectID: subjectID,
      name: name,
      parent: parent,
      recursionDepth: recursionDepth
    )
    let attempt = role == .attempt ? parent!.nextAttempt(for: name) : 1
    let started = runtime?.timePoint()
    let traceStartedMonotonicNano = DispatchTime.now().uptimeNanoseconds
    let semanticContext = SemanticTaskContext.current
    let traceSpan = ChillTraceRuntime.startSpan(
      name: name.rawValue,
      kind: .internal,
      attributes: [
        "chill.activity.id": .string(subjectID.rawValue),
        "chill.activity.attempt": .integer(Int64(attempt)),
        "chill.activity.recursion_depth": .integer(Int64(recursionDepth)),
      ]
    )
    if let runtime, let started {
      try! runtime.record(captureClass: .analytics, at: started) {
        try BehaviorDraft(
          subjectID: subjectID,
          kind: .activity,
          operation: .start,
          name: name,
          annotations: semanticContext.annotations,
          page: semanticContext.page,
          trace: traceSpan.context,
          payload: .activity(
            ActivityPayload(
              kind: kind,
              role: role,
              parentActivityID: parent?.subjectID,
              attempt: attempt,
              recursionDepth: recursionDepth
            )
          )
        )
      }
    }
    return ActivityToken(
      runtime: runtime,
      frame: frame,
      kind: kind,
      role: role,
      attempt: attempt,
      started: started,
      traceStartedMonotonicNano: traceStartedMonotonicNano,
      semanticContext: semanticContext,
      traceSpan: traceSpan
    )
  }

  private static func eventToken(
    name rawName: String,
    eventClass: EventClass,
    severity: EventSeverity,
    emission: EventEmission,
    deduplicationKey: String?
  ) -> EventToken? {
    guard emission != .observed,
      let runtime = Chill.currentRuntime(),
      runtime.isCaptureEnabled(for: .analytics),
      let name = try? SemanticName(rawName)
    else {
      return nil
    }
    return EventToken(
      runtime: runtime,
      name: name,
      eventClass: eventClass,
      severity: severity,
      emission: emission,
      deduplicationKey: deduplicationKey,
      semanticContext: SemanticTaskContext.current
    )
  }
}
