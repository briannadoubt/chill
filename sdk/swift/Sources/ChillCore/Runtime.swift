import os

public struct RuntimeConfiguration: Equatable, Sendable {
  public let enabled: Bool
  public let sessionID: SessionID
  public let bootID: BootID
  public let privacy: PrivacyPolicy
  public let sampling: SamplingConfiguration

  public init(
    enabled: Bool = true,
    sessionID: SessionID,
    bootID: BootID,
    privacy: PrivacyPolicy,
    sampling: SamplingConfiguration
  ) {
    self.enabled = enabled
    self.sessionID = sessionID
    self.bootID = bootID
    self.privacy = privacy
    self.sampling = sampling
  }
}

public struct BehaviorRecord: Equatable, Sendable {
  public let schemaVersion: String
  public let schemaURL: String
  public let recordID: RecordID
  public let subjectID: SubjectID
  public let sessionID: SessionID
  public let kind: BehaviorKind
  public let operation: BehaviorOperation
  public let name: SemanticName
  public let clock: RecordClock
  public let annotations: AnnotationSnapshot
  public let page: PagePath?
  public let element: ElementIdentity?
  public let trace: TraceContext?
  public let captureClass: CaptureClass
  public let consent: ConsentState
  public let policyVersion: String
  public let redactionState: RedactionState
  public let redactionCount: UInt32
  public let payload: BehaviorPayload
  public let durationNano: UInt64?
}

package struct BehaviorDraft: Sendable {
  package let subjectID: SubjectID
  package let kind: BehaviorKind
  package let operation: BehaviorOperation
  package let name: SemanticName
  package let annotations: AnnotationSnapshot
  package let page: PagePath?
  package let element: ElementIdentity?
  package let trace: TraceContext?
  package let redactionState: RedactionState
  package let redactionCount: UInt32
  package let payload: BehaviorPayload
  package let durationNano: UInt64?

  package init(
    subjectID: SubjectID,
    kind: BehaviorKind,
    operation: BehaviorOperation,
    name: SemanticName,
    annotations: AnnotationSnapshot = .empty,
    page: PagePath? = nil,
    element: ElementIdentity? = nil,
    trace: TraceContext? = nil,
    redactionState: RedactionState = .none,
    redactionCount: UInt32 = 0,
    payload: BehaviorPayload = .none,
    durationNano: UInt64? = nil
  ) throws {
    let operationIsValid =
      switch kind {
      case .session, .journey, .page:
        operation != .instant
      case .activity:
        operation == .start || operation == .end
      case .impression, .action, .event, .replay:
        operation == .instant
      }
    guard operationIsValid else {
      throw ContractError.invalidBehaviorOperation
    }
    let payloadMatches =
      switch (kind, payload) {
      case (.page, .page(_)),
        (.impression, .impression(_)),
        (.action, .action(_)),
        (.activity, .activity(_)),
        (.event, .event(_)),
        (.replay, .replay(_)):
        true
      case (.page, _), (.impression, _), (.action, _), (.activity, _),
        (.event, _), (.replay, _):
        false
      default:
        true
      }
    guard payloadMatches else {
      throw ContractError.payloadKindMismatch
    }
    self.subjectID = subjectID
    self.kind = kind
    self.operation = operation
    self.name = name
    self.annotations = annotations
    self.page = page
    self.element = element
    self.trace = trace ?? TraceTaskContext.current
    self.redactionState = redactionState
    self.redactionCount = redactionCount
    self.payload = payload
    self.durationNano = durationNano
  }
}

/// A sink must enqueue synchronously without file, database, or network I/O.
/// Delivery targets own their worker and backpressure policy.
public protocol RecordSink: Sendable {
  func submit(_ record: BehaviorRecord)
}

public struct NoOpRecordSink: RecordSink {
  public init() {}
  public func submit(_: BehaviorRecord) {}
}

public final class ChillRuntime: Sendable {
  private struct State: Sendable {
    var sequenceNumber: UInt64 = 0
    var seenEventKeys: Set<String> = []
    var privacy: PrivacyPolicy
    var collectionState: RemoteCollectionState? = nil
  }

  public let configuration: RuntimeConfiguration
  private let clock: any ChillClock
  private let idGenerator: any RecordIDGenerating
  private let sink: any RecordSink
  private let state: OSAllocatedUnfairLock<State>
  private let behaviorSampled: Bool
  private let replaySampled: Bool

  public convenience init(
    configuration: RuntimeConfiguration,
    sink: any RecordSink,
    clock: any ChillClock = SystemChillClock()
  ) {
    self.init(
      configuration: configuration,
      sink: sink,
      clock: clock,
      idGenerator: SystemRecordIDGenerator()
    )
  }

  package init(
    configuration: RuntimeConfiguration,
    sink: any RecordSink,
    clock: any ChillClock,
    idGenerator: any RecordIDGenerating
  ) {
    self.configuration = configuration
    self.sink = sink
    self.clock = clock
    self.idGenerator = idGenerator
    state = OSAllocatedUnfairLock(
      initialState: State(privacy: configuration.privacy)
    )
    if configuration.enabled {
      let sampler = try! DeterministicSampler(
        salt: configuration.sampling.salt
      )
      behaviorSampled =
        sampler.decide(
          stream: .behavior,
          stableKey: configuration.sampling.stableKey,
          rate: configuration.sampling.behavior
        ).kept
      replaySampled =
        sampler.decide(
          stream: .replay,
          stableKey: configuration.sampling.stableKey,
          rate: configuration.sampling.replay
        ).kept
    } else {
      behaviorSampled = false
      replaySampled = false
    }
  }

  @inline(__always)
  public func isCaptureEnabled(for captureClass: CaptureClass) -> Bool {
    guard activePrivacy(for: captureClass) != nil else { return false }
    return samplingAllows(captureClass)
  }

  /// Applies local consent and policy changes immediately. Remote collection
  /// state can still narrow this policy, but can never grant local consent.
  public func updatePrivacyPolicy(_ privacy: PrivacyPolicy) {
    state.withLock { $0.privacy = privacy }
  }

  /// Applies only a newer server revision. A policy-version mismatch fails
  /// closed until the SDK receives a matching local privacy policy.
  @discardableResult
  public func applyCollectionState(
    _ collectionState: RemoteCollectionState
  ) -> CollectionStateApplyResult {
    state.withLock { state in
      if let current = state.collectionState,
        collectionState.revision <= current.revision
      {
        return .stale
      }
      state.collectionState = collectionState
      return .applied
    }
  }

  public var currentCollectionState: RemoteCollectionState? {
    state.withLock { $0.collectionState }
  }

  @inline(__always)
  private func activePrivacy(
    for captureClass: CaptureClass
  ) -> PrivacyPolicy? {
    guard configuration.enabled else { return nil }
    return state.withLock {
      activePrivacy(in: $0, for: captureClass)
    }
  }

  @inline(__always)
  private func activePrivacy(
    in state: State,
    for captureClass: CaptureClass
  ) -> PrivacyPolicy? {
    guard state.privacy.allows(captureClass) else { return nil }
    guard let remote = state.collectionState else { return state.privacy }
    guard remote.enabled,
      remote.policyVersion == state.privacy.version,
      !remote.disabledCaptureClasses.contains(captureClass)
    else { return nil }
    return state.privacy
  }

  @inline(__always)
  private func samplingAllows(_ captureClass: CaptureClass) -> Bool {
    switch captureClass {
    case .essential:
      return true
    case .analytics, .diagnostic:
      return behaviorSampled
    case .replay:
      return replaySampled
    }
  }

  package func timePoint() -> TimePoint {
    clock.now()
  }

  package func claimEvent(name: SemanticName, keyHash: String?) -> Bool {
    guard let keyHash else { return true }
    return state.withLock { state in
      state.seenEventKeys.insert("\(name.rawValue)\0\(keyHash)").inserted
    }
  }

  /// Package-only so adapters and narrow generated-code support wrappers can
  /// record facts while application developers receive no tracking API.
  @inline(__always)
  @discardableResult
  package func record(
    captureClass: CaptureClass,
    at suppliedTime: TimePoint? = nil,
    _ makeDraft: () throws -> BehaviorDraft
  ) rethrows -> Bool {
    guard activePrivacy(for: captureClass) != nil,
      samplingAllows(captureClass)
    else { return false }

    let draft = try makeDraft()
    let time = suppliedTime ?? clock.now()
    return state.withLock { state in
      // Recheck while serializing the final queue submission with policy
      // updates. Once a withdrawal returns, no older grant can submit later.
      guard let privacy = activePrivacy(in: state, for: captureClass) else {
        return false
      }
      let classifiedAnnotations = privacy.classifyAnnotations(
        draft.annotations
      )
      state.sequenceNumber &+= 1
      let identity = (idGenerator.nextRecordID(), state.sequenceNumber)
      // Instant facts have no lifecycle identity separate from the immutable
      // record itself. Keeping both IDs identical makes retries idempotent and
      // prevents downstream normalizers from inventing a subject.
      let subjectID =
        switch draft.kind {
        case .impression, .action, .event:
          try! SubjectID(identity.0.rawValue)
        case .session, .journey, .page, .activity, .replay:
          draft.subjectID
        }
      let record = BehaviorRecord(
        schemaVersion: "1.0.0",
        schemaURL: "https://schemas.chill.dev/behavior/v1/envelope.schema.json",
        recordID: identity.0,
        subjectID: subjectID,
        sessionID: configuration.sessionID,
        kind: draft.kind,
        operation: draft.operation,
        name: draft.name,
        clock: RecordClock(
          occurredAtUnixNano: time.occurredAtUnixNano,
          observedAtUnixNano: time.occurredAtUnixNano,
          monotonicNano: time.monotonicNano,
          bootID: configuration.bootID,
          sequenceNumber: identity.1
        ),
        annotations: classifiedAnnotations.snapshot,
        page: draft.page,
        element: draft.element,
        trace: draft.trace,
        captureClass: captureClass,
        consent: privacy.consentState(for: captureClass),
        policyVersion: privacy.version,
        redactionState:
          classifiedAnnotations.omitted > 0 && draft.redactionState == .none
          ? .applied : draft.redactionState,
        redactionCount: UInt32(
          clamping: UInt64(draft.redactionCount)
            + UInt64(classifiedAnnotations.omitted)
        ),
        payload: draft.payload,
        durationNano: draft.durationNano
      )
      sink.submit(record)
      return true
    }
  }
}
