import Chill
import Testing
import os

private final class MacroCollectingSink: RecordSink {
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

private enum MacroProbeError: Error, Equatable {
  case expected
}

@Activity("generic.echo")
private func instrumentedEcho<Value: Sendable>(_ value: Value) -> Value {
  value
}

@Activity("activity.typed_failure")
private func typedFailure() throws(MacroProbeError) -> Int {
  throw .expected
}

@Activity("activity.cancel")
private func cancelledActivity() async throws -> Never {
  throw CancellationError()
}

@Activity("activity.recursive", kind: .task)
private func recursiveActivity(_ remaining: Int) -> Int {
  remaining == 0 ? 0 : 1 + recursiveActivity(remaining - 1)
}

@Activity("repository.load")
private func loadWithAttempts() -> Bool {
  _ = networkAttempt(succeeds: false)
  return networkAttempt(succeeds: true)
}

@Activity("network.fetch", kind: .network, role: .attempt)
private func networkAttempt(succeeds: Bool) -> Bool {
  succeeds
}

@Event("cat.loaded")
private func successfulEvent() -> String {
  "cat"
}

@Event("cat.loaded")
private func unsuccessfulEvent() throws(MacroProbeError) {
  throw .expected
}

@Event(
  "cat.terminal",
  eventClass: .error,
  severity: .error,
  emission: .terminal
)
private func terminalEvent() throws(MacroProbeError) {
  throw .expected
}

@Event("cat.entered", emission: .entered)
private func enteredEvent() {}

@Event("cat.unique", deduplicationKey: "cat-42")
private func deduplicatedEvent() {}

@MainActor
@Activity("ui.main_actor", kind: .ui)
private func mainActorActivity() -> String {
  "isolated"
}

@Activity("disabled.body")
private func disabledBody(_ executions: inout Int) {
  executions += 1
}

@Suite("Compiled Chill macro behavior", .serialized)
struct MacroIntegrationTests {
  @Test("Activity preserves generic results and emits paired lifecycle facts")
  func activityLifecycle() throws {
    let sink = try configureRuntime()
    defer { Chill.disable() }

    #expect(instrumentedEcho(42) == 42)

    let records = sink.records
    #expect(records.count == 2)
    #expect(records.map(\.operation) == [.start, .end])
    #expect(records.map(\.name.rawValue) == ["generic.echo", "generic.echo"])
    #expect(records[0].subjectID == records[1].subjectID)
    #expect(records[0].trace != nil)
    #expect(records[0].trace == records[1].trace)
    #expect(activityPayload(records[0])?.outcome == nil)
    #expect(activityPayload(records[1])?.outcome == .succeeded)
    #expect(activityPayload(records[1])?.durationNano != nil)
  }

  @Test("Typed errors and cancellation become exact terminal outcomes")
  func terminalOutcomes() async throws {
    let sink = try configureRuntime()
    defer { Chill.disable() }

    do {
      _ = try typedFailure()
      Issue.record("typedFailure should throw")
    } catch {
      #expect(error == .expected)
    }
    do {
      try await cancelledActivity()
    } catch {
      #expect(error is CancellationError)
    }

    let terminal = sink.records.filter { $0.operation == .end }
    #expect(terminal.count == 2)
    #expect(activityPayload(terminal[0])?.outcome == .failed)
    #expect(activityPayload(terminal[1])?.outcome == .cancelled)
  }

  @Test("Recursive calls carry deterministic depth")
  func recursionDepth() throws {
    let sink = try configureRuntime()
    defer { Chill.disable() }

    #expect(recursiveActivity(2) == 2)

    let starts = sink.records.filter { $0.operation == .start }
    #expect(
      starts.compactMap { activityPayload($0)?.recursionDepth } == [0, 1, 2]
    )
    #expect(starts[1].subjectID != starts[0].subjectID)
    #expect(starts[1].trace?.traceID == starts[0].trace?.traceID)
    #expect(starts[1].trace?.parentSpanID == starts[0].trace?.spanID)
    #expect(starts[2].trace?.parentSpanID == starts[1].trace?.spanID)
    #expect(
      activityPayload(starts[1])?.parentActivityID == starts[0].subjectID
    )
    #expect(
      activityPayload(starts[2])?.parentActivityID == starts[1].subjectID
    )
  }

  @Test("Attempt activities are numbered within their operation")
  func attemptNumbering() throws {
    let sink = try configureRuntime()
    defer { Chill.disable() }

    #expect(loadWithAttempts())

    let starts = sink.records.filter { $0.operation == .start }
    #expect(starts.count == 3)
    let attempts = starts.filter {
      activityPayload($0)?.role == .attempt
    }
    #expect(attempts.compactMap { activityPayload($0)?.attempt } == [1, 2])
    #expect(
      attempts.allSatisfy {
        activityPayload($0)?.parentActivityID == starts[0].subjectID
      }
    )
  }

  @Test("Event lifecycle, failure policy, and deduplication are declarative")
  func eventBehavior() throws {
    let sink = try configureRuntime()
    defer { Chill.disable() }

    #expect(successfulEvent() == "cat")
    do {
      try unsuccessfulEvent()
      Issue.record("unsuccessfulEvent should throw")
    } catch {
      #expect(error == .expected)
    }
    do {
      try terminalEvent()
      Issue.record("terminalEvent should throw")
    } catch {
      #expect(error == .expected)
    }
    enteredEvent()
    deduplicatedEvent()
    deduplicatedEvent()

    let events = sink.records.filter { $0.kind == .event }
    #expect(
      events.map(\.name.rawValue) == [
        "cat.loaded",
        "cat.terminal",
        "cat.entered",
        "cat.unique",
      ]
    )
    #expect(eventPayload(events[0])?.outcome == .succeeded)
    #expect(eventPayload(events[1])?.outcome == .failed)
    #expect(eventPayload(events[2])?.outcome == nil)
    #expect(
      eventPayload(events[3])?.deduplicationKeyHash
        == "4bf7847f5b2c3cdd7038ea51f25ecd6e73947ce6c0c9321159d1393acde6537c"
    )
  }

  @Test("Body macros preserve global-actor isolation")
  @MainActor
  func actorIsolation() throws {
    let sink = try configureRuntime()
    defer { Chill.disable() }

    #expect(mainActorActivity() == "isolated")
    #expect(sink.records.count == 2)
  }

  @Test("Disabled instrumentation executes the body without records")
  func disabledFastPath() {
    Chill.disable()
    var executions = 0

    disabledBody(&executions)

    #expect(executions == 1)
    #expect(!Chill.isEnabled)
  }
}

private func configureRuntime() throws -> MacroCollectingSink {
  Chill.disable()
  let sink = MacroCollectingSink()
  let runtime = ChillRuntime(
    configuration: try RuntimeConfiguration(
      sessionID: SessionID("macro-session"),
      bootID: BootID("macro-boot"),
      privacy: PrivacyPolicy(
        version: "macro-privacy-v1",
        analytics: .granted
      ),
      sampling: SamplingConfiguration(
        stableKey: "macro-user",
        salt: "macro-salt"
      )
    ),
    sink: sink
  )
  Chill.configure(runtime)
  return sink
}

private func activityPayload(_ record: BehaviorRecord) -> ActivityPayload? {
  guard case .activity(let payload) = record.payload else { return nil }
  return payload
}

private func eventPayload(_ record: BehaviorRecord) -> EventPayload? {
  guard case .event(let payload) = record.payload else { return nil }
  return payload
}
