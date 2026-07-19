import Testing

@testable import ChillCore

@Test
func lifecycleEndsExactlyOnceWithMonotonicDuration() {
  var state = LifecycleState()

  #expect(state.begin(at: 1_000) == .started)
  #expect(state.begin(at: 1_001) == .ignored(.duplicateStart))
  #expect(
    state.finish(at: 1_750, outcome: .succeeded)
      == .ended(outcome: .succeeded, durationNano: 750)
  )
  #expect(
    state.finish(at: 1_800, outcome: .failed)
      == .ignored(.duplicateTerminal)
  )
  #expect(
    state.phase
      == .terminal(outcome: .succeeded, durationNano: 750)
  )
}

@Test
func lifecycleRejectsUnknownAndBackwardsTerminalCallbacks() {
  var idle = LifecycleState()
  #expect(
    idle.finish(at: 10, outcome: .failed)
      == .ignored(.unknownTerminal)
  )

  var active = LifecycleState()
  active.begin(at: 100)
  #expect(
    active.finish(at: 99, outcome: .failed)
      == .ignored(.monotonicTimeMovedBackwards)
  )
  #expect(active.phase == .active(startedMonotonicNano: 100))
}
