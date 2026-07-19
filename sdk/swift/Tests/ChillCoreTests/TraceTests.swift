import Testing
import os

@_spi(TraceIntegration) @testable import ChillCore

private final class TraceTestSink: RecordSink {
  private let state = OSAllocatedUnfairLock(initialState: 0)
  func submit(_: BehaviorRecord) { state.withLock { $0 += 1 } }
  var count: Int { state.withLock { $0 } }
}

private final class TraceTestHandle: ChillTraceSpanHandle {
  let context: TraceContext
  private let state: OSAllocatedUnfairLock<Int>

  init(context: TraceContext, state: OSAllocatedUnfairLock<Int>) {
    self.context = context
    self.state = state
  }

  func setAttributes(_: [String: ChillTraceAttributeValue]) {}
  func end(status _: ChillTraceSpanStatus) { state.withLock { $0 += 1 } }
}

private final class TraceTestProvider: ChillTraceProvider {
  private let starts = OSAllocatedUnfairLock(initialState: 0)
  private let ends = OSAllocatedUnfairLock(initialState: 0)

  func startSpan(_ request: ChillTraceSpanRequest) -> any ChillTraceSpanHandle {
    starts.withLock { $0 += 1 }
    return TraceTestHandle(
      context: .localChild(of: request.parent),
      state: ends
    )
  }

  var startCount: Int { starts.withLock { $0 } }
  var endCount: Int { ends.withLock { $0 } }
}

@Suite("W3C trace context", .serialized)
struct TraceTests {
  private let context = try! TraceContext(
    traceID: "4bf92f3577b34da6a3ce929d0e0e4736",
    spanID: "00f067aa0ba902b7",
    traceFlags: 1,
    traceState: "vendor=value"
  )

  @Test
  func traceParentRoundTripsExactly() {
    let header = W3CTraceContext.traceParent(for: context)
    #expect(
      header
        == "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    )
    let extracted = W3CTraceContext.extract(
      traceParent: header,
      traceState: context.traceState
    )
    #expect(extracted?.traceID == context.traceID)
    #expect(extracted?.spanID == context.spanID)
    #expect(extracted?.traceFlags == 1)
    #expect(extracted?.traceState == "vendor=value")
    #expect(extracted?.isRemote == true)
  }

  @Test(
    arguments: [
      "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
      "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
      "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
      "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-0A",
      "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
      "00-too-short-00f067aa0ba902b7-01",
    ]
  )
  func invalidTraceParentsRestartInsteadOfPartiallyTrusting(_ value: String) {
    #expect(W3CTraceContext.extract(traceParent: value) == nil)
  }

  @Test(
    arguments: [
      "UPPER=value",
      "vendor=value,vendor=duplicate",
      "vendor=trailing ",
      "vendor=header\r\ninjection",
      "vendor=value=extra",
    ]
  )
  func traceStateRejectsNoncanonicalOrInjectableValues(_ value: String) {
    #expect(throws: TraceContractError.invalidTraceState) {
      _ = try TraceContext(
        traceID: context.traceID,
        spanID: context.spanID,
        traceState: value
      )
    }
  }

  @Test
  func behaviorDraftCapturesTaskLocalContextAutomatically() throws {
    let draft = try TraceTaskContext.$current.withValue(context) {
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
    #expect(draft.trace == context)
  }

  @Test
  func generatedChildrenRetainTraceAndUseIndependentSpanIDs() {
    let child = TraceContext.localChild(of: context)
    #expect(child.traceID == context.traceID)
    #expect(child.parentSpanID == context.spanID)
    #expect(child.spanID != context.spanID)
    #expect(child.traceFlags == context.traceFlags)
  }

  @Test
  func declarativeActionDispatchIsNoOpWhenTracingIsUnconfigured() throws {
    Chill.disable()
    ChillTraceRuntime.resetProvider()
    let captured = try _ChillInstrumentation.withActionTrace(
      name: SemanticName("cat.adopt")
    ) {
      TraceTaskContext.current
    }
    #expect(captured == nil)
  }

  @Test
  func activityTracingIsIndependentFromBehaviorSampling() throws {
    let sink = TraceTestSink()
    let runtime = ChillRuntime(
      configuration: try RuntimeConfiguration(
        sessionID: SessionID("trace-session"),
        bootID: BootID("trace-boot"),
        privacy: PrivacyPolicy(
          version: "trace-privacy-v1",
          analytics: .granted
        ),
        sampling: SamplingConfiguration(
          stableKey: "trace-user",
          salt: "trace-salt",
          behavior: .none
        )
      ),
      sink: sink
    )
    let provider = TraceTestProvider()
    Chill.configure(runtime)
    Chill.configureTraceProvider(provider)
    defer {
      Chill.disable()
      ChillTraceRuntime.resetProvider()
    }

    let value = _ChillInstrumentation.withActivity(
      name: "repository.sampled_out",
      kind: .domain,
      role: .operation
    ) { 42 }

    #expect(value == 42)
    #expect(sink.count == 0)
    #expect(provider.startCount == 1)
    #expect(provider.endCount == 1)
  }
}
