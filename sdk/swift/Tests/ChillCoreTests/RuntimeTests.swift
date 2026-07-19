import Testing
import os

@testable import ChillCore

private final class TestClock: ChillClock {
  private let calls = OSAllocatedUnfairLock(initialState: 0)

  func now() -> TimePoint {
    let call = calls.withLock { value in
      value += 1
      return value
    }
    return TimePoint(
      occurredAtUnixNano: 1_700_000_000_000_000_000 + UInt64(call),
      monotonicNano: 1_000 + UInt64(call)
    )
  }

  var callCount: Int { calls.withLock { $0 } }
}

private final class TestIDGenerator: RecordIDGenerating {
  private let calls = OSAllocatedUnfairLock(initialState: 0)

  func nextRecordID() -> RecordID {
    let call = calls.withLock { value in
      value += 1
      return value
    }
    return try! RecordID("record-\(call)")
  }

  var callCount: Int { calls.withLock { $0 } }
}

private final class CollectingSink: RecordSink {
  private let storage = OSAllocatedUnfairLock(initialState: [BehaviorRecord]())

  func submit(_ record: BehaviorRecord) {
    storage.withLock { $0.append(record) }
  }

  var records: [BehaviorRecord] { storage.withLock { $0 } }
}

private final class Counter: Sendable {
  private let value = OSAllocatedUnfairLock(initialState: 0)

  func increment() { value.withLock { $0 += 1 } }
  var count: Int { value.withLock { $0 } }
}

private func configuration(
  enabled: Bool = true,
  analytics: ConsentState = .granted,
  replay: ConsentState = .denied,
  behaviorRate: SamplingRate = .all,
  replayRate: SamplingRate = .none,
  annotationAllowlist: [String: DataClassification] = [:]
) throws -> RuntimeConfiguration {
  try RuntimeConfiguration(
    enabled: enabled,
    sessionID: SessionID("session-1"),
    bootID: BootID("boot-1"),
    privacy: PrivacyPolicy(
      version: "privacy-v1",
      analytics: analytics,
      diagnostic: analytics,
      replay: replay,
      annotationAllowlist: annotationAllowlist
    ),
    sampling: SamplingConfiguration(
      stableKey: "user-2",
      salt: "test-salt-v1",
      behavior: behaviorRate,
      replay: replayRate
    )
  )
}

@Test
func runtimeRedactsUnclassifiedAnnotationsBeforeTheSink() throws {
  let sink = CollectingSink()
  let runtime = ChillRuntime(
    configuration: try configuration(
      annotationAllowlist: [
        "cat.id": .pseudonymousIdentifier,
        "private.note": .sensitiveData,
      ]
    ),
    sink: sink
  )
  let annotations = try AnnotationContext().addingScope(
    id: AnnotationScopeID("runtime-privacy"),
    declarations: [
      AnnotationDeclaration("cat.id", value: "cat-42"),
      AnnotationDeclaration("private.note", value: "never serialize"),
      AnnotationDeclaration("unknown.value", value: "default denied"),
    ]
  ).snapshot

  _ = try runtime.record(captureClass: .analytics) {
    try BehaviorDraft(
      subjectID: SubjectID("event-privacy"),
      kind: .event,
      operation: .instant,
      name: SemanticName("privacy.checked"),
      annotations: annotations,
      payload: .event(
        EventPayload(
          eventClass: .domain,
          severity: .info,
          emission: .observed
        )
      )
    )
  }

  let record = try #require(sink.records.first)
  #expect(record.annotations.values.count == 1)
  #expect(
    record.annotations.values[try AnnotationName("cat.id")]
      == .string("cat-42")
  )
  #expect(record.redactionState == .applied)
  #expect(record.redactionCount == 2)
}

private func eventDraft() throws -> BehaviorDraft {
  try BehaviorDraft(
    subjectID: SubjectID("event-1"),
    kind: .event,
    operation: .instant,
    name: SemanticName("cat.adopted"),
    payload: .event(
      EventPayload(
        eventClass: .domain,
        severity: .info,
        emission: .succeeded,
        outcome: .succeeded
      )
    )
  )
}

private func replayDraft() throws -> BehaviorDraft {
  try BehaviorDraft(
    subjectID: SubjectID("replay-1"),
    kind: .replay,
    operation: .instant,
    name: SemanticName("session.replay"),
    payload: .replay(
      ReplayPayload(
        replayID: "replay-1",
        chunkID: "chunk-1",
        chunkIndex: 0,
        startsAtUnixNano: 100,
        endsAtUnixNano: 200,
        sha256: String(repeating: "a", count: 64),
        byteCount: 128,
        storageRef: "replay://chunk/chunk-1"
      )
    )
  )
}

@Test
func disabledRuntimeDoesNoCaptureWork() throws {
  let clock = TestClock()
  let ids = TestIDGenerator()
  let sink = CollectingSink()
  let builds = Counter()
  let runtime = ChillRuntime(
    configuration: try configuration(enabled: false),
    sink: sink,
    clock: clock,
    idGenerator: ids
  )

  let emitted = try runtime.record(captureClass: .analytics) {
    builds.increment()
    return try eventDraft()
  }

  #expect(!emitted)
  #expect(builds.count == 0)
  #expect(clock.callCount == 0)
  #expect(ids.callCount == 0)
  #expect(sink.records.isEmpty)
}

@Test
func deniedOrSampledOutCaptureDoesNoCaptureWork() throws {
  let cases: [(ConsentState, SamplingRate)] = [
    (.denied, .all),
    (.granted, .none),
  ]
  for (consent, rate) in cases {
    let clock = TestClock()
    let ids = TestIDGenerator()
    let sink = CollectingSink()
    let builds = Counter()
    let runtime = ChillRuntime(
      configuration: try configuration(
        analytics: consent,
        behaviorRate: rate
      ),
      sink: sink,
      clock: clock,
      idGenerator: ids
    )

    _ = try runtime.record(captureClass: .analytics) {
      builds.increment()
      return try eventDraft()
    }

    #expect(builds.count == 0)
    #expect(clock.callCount == 0)
    #expect(ids.callCount == 0)
    #expect(sink.records.isEmpty)
  }
}

@Test
func enabledRuntimeBuildsCanonicalRecordAfterItsGates() throws {
  let clock = TestClock()
  let ids = TestIDGenerator()
  let sink = CollectingSink()
  let runtime = ChillRuntime(
    configuration: try configuration(),
    sink: sink,
    clock: clock,
    idGenerator: ids
  )

  #expect(try runtime.record(captureClass: .analytics) { try eventDraft() })
  #expect(try runtime.record(captureClass: .analytics) { try eventDraft() })

  let records = sink.records
  #expect(records.map(\.recordID.rawValue) == ["record-1", "record-2"])
  #expect(records.map(\.clock.sequenceNumber) == [1, 2])
  #expect(records[0].schemaVersion == "1.0.0")
  #expect(
    records[0].schemaURL
      == "https://schemas.chill.dev/behavior/v1/envelope.schema.json"
  )
  #expect(records[0].captureClass == .analytics)
  #expect(records[0].consent == .granted)
  #expect(records[0].policyVersion == "privacy-v1")
}

@Test
func liveConsentAndRemoteCollectionStateFailClosed() throws {
  let sink = CollectingSink()
  let runtime = ChillRuntime(
    configuration: try configuration(analytics: .unknown),
    sink: sink
  )

  #expect(!runtime.isCaptureEnabled(for: .analytics))
  runtime.updatePrivacyPolicy(
    try PrivacyPolicy(
      version: "privacy-v1",
      analytics: .granted,
      diagnostic: .granted
    )
  )
  #expect(runtime.isCaptureEnabled(for: .analytics))

  let disabled = try RemoteCollectionState(
    revision: 2,
    policyVersion: "privacy-v1",
    enabled: true,
    disabledCaptureClasses: [.analytics],
    effectiveAtUnixNano: 100
  )
  #expect(runtime.applyCollectionState(disabled) == .applied)
  #expect(!runtime.isCaptureEnabled(for: .analytics))

  let stale = try RemoteCollectionState(
    revision: 1,
    policyVersion: "privacy-v1",
    enabled: true,
    disabledCaptureClasses: [],
    effectiveAtUnixNano: 90
  )
  #expect(runtime.applyCollectionState(stale) == .stale)
  #expect(!runtime.isCaptureEnabled(for: .analytics))

  let mismatched = try RemoteCollectionState(
    revision: 3,
    policyVersion: "privacy-v2",
    enabled: true,
    disabledCaptureClasses: [],
    effectiveAtUnixNano: 110
  )
  #expect(runtime.applyCollectionState(mismatched) == .applied)
  #expect(!runtime.isCaptureEnabled(for: .essential))

  runtime.updatePrivacyPolicy(
    try PrivacyPolicy(
      version: "privacy-v2",
      analytics: .granted,
      diagnostic: .granted
    )
  )
  #expect(runtime.isCaptureEnabled(for: .analytics))
  #expect(try runtime.record(captureClass: .analytics) { try eventDraft() })
  #expect(sink.records.last?.policyVersion == "privacy-v2")
  #expect(sink.records.last?.consent == .granted)
}

@Test
func captureClassesAndSamplingStreamsRemainIndependent() throws {
  let sink = CollectingSink()
  let runtime = ChillRuntime(
    configuration: try configuration(
      analytics: .denied,
      replay: .granted,
      behaviorRate: .none,
      replayRate: .all
    ),
    sink: sink
  )

  let analyticsEmitted = try runtime.record(captureClass: .analytics) {
    try eventDraft()
  }
  let replayEmitted = try runtime.record(captureClass: .replay) {
    try replayDraft()
  }
  let essentialEmitted = try runtime.record(captureClass: .essential) {
    try eventDraft()
  }
  #expect(!analyticsEmitted)
  #expect(replayEmitted)
  #expect(essentialEmitted)
  #expect(sink.records.map(\.captureClass) == [.replay, .essential])
}

@Test
func runtimeSequenceIsUniqueUnderConcurrency() async throws {
  let sink = CollectingSink()
  let runtime = ChillRuntime(
    configuration: try configuration(),
    sink: sink
  )

  await withTaskGroup(of: Void.self) { group in
    for _ in 0..<500 {
      group.addTask {
        _ = try! runtime.record(captureClass: .analytics) {
          try eventDraft()
        }
      }
    }
  }

  let records = sink.records
  let sequence = Set(records.map(\.clock.sequenceNumber))
  let ids = Set(records.map(\.recordID))
  #expect(records.count == 500)
  #expect(sequence.count == 500)
  #expect(sequence.min() == 1)
  #expect(sequence.max() == 500)
  #expect(ids.count == 500)
}

@Test
func draftRejectsKindOperationAndPayloadMismatches() throws {
  #expect(throws: ContractError.self) {
    _ = try BehaviorDraft(
      subjectID: SubjectID("action-1"),
      kind: .action,
      operation: .start,
      name: SemanticName("cat.adopt"),
      payload: .action(
        ActionPayload(
          elementID: SemanticName("cat.adopt"),
          role: SemanticName("button"),
          activation: .primary,
          input: .touch
        )
      )
    )
  }
  #expect(throws: ContractError.self) {
    _ = try BehaviorDraft(
      subjectID: SubjectID("event-1"),
      kind: .event,
      operation: .instant,
      name: SemanticName("cat.adopted")
    )
  }
}

@Test
func pageAndImpressionRatiosAreFiniteAndBounded() throws {
  let path = try PagePath(
    surfaceID: SurfaceID("surface-1"),
    instanceID: PageInstanceID("page-1"),
    instanceIDs: [PageInstanceID("page-1")],
    segments: [PageSegment("cat")],
    relation: .root,
    exposure: .foreground,
    focused: true
  )
  #expect(throws: ContractError.invalidVisibilityRatio) {
    _ = try PagePayload(
      path: path,
      cause: .initial,
      visibilityRatio: 1.01
    )
  }
  #expect(throws: ContractError.invalidVisibilityRatio) {
    _ = try ImpressionPayload(
      elementID: SemanticName("cat.hero"),
      role: SemanticName("heading"),
      visibilityRatio: .nan
    )
  }
}
