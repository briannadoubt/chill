import Foundation
import os

package struct TimedReplayGesture: Sendable {
  package let gesture: ReplayGesture
  package let occurredAtUnixNano: UInt64
  package let monotonicNano: UInt64
}

package final class ReplayAdmission: @unchecked Sendable {
  private struct State {
    var latestObservation: ReplayObservation?
    var gestures: [TimedReplayGesture] = []
    var drainScheduled = false
    var coalesced: UInt64 = 0
    var waiters: [CheckedContinuation<Void, Never>] = []
  }

  private enum Event {
    case observation(ReplayObservation)
    case gesture(TimedReplayGesture)

    var monotonicNano: UInt64 {
      switch self {
      case .observation(let value): value.monotonicNano
      case .gesture(let value): value.monotonicNano
      }
    }
  }

  private struct Batch {
    let events: [Event]
    let coalesced: UInt64
  }

  private enum DrainStep {
    case batch(Batch)
    case finished([CheckedContinuation<Void, Never>])
  }

  private let state = OSAllocatedUnfairLock(initialState: State())
  private let engine: ReplayEngine

  package init(engine: ReplayEngine) {
    self.engine = engine
  }

  package func submit(_ observation: ReplayObservation) {
    let shouldSchedule = state.withLock { state -> Bool in
      if state.latestObservation != nil { state.coalesced &+= 1 }
      state.latestObservation = observation
      guard !state.drainScheduled else { return false }
      state.drainScheduled = true
      return true
    }
    if shouldSchedule { scheduleDrain() }
  }

  package func submit(_ gesture: TimedReplayGesture) {
    let shouldSchedule = state.withLock { state -> Bool in
      if state.gestures.count == 64 {
        state.gestures.removeFirst()
        state.coalesced &+= 1
      }
      state.gestures.append(gesture)
      guard !state.drainScheduled else { return false }
      state.drainScheduled = true
      return true
    }
    if shouldSchedule { scheduleDrain() }
  }

  package func flush() async {
    await waitForDrain()
    await engine.flush()
  }

  package func discardUnsealed() async {
    state.withLock { state in
      if state.latestObservation != nil { state.coalesced &+= 1 }
      state.coalesced &+= UInt64(state.gestures.count)
      state.latestObservation = nil
      state.gestures.removeAll(keepingCapacity: true)
    }
    await waitForDrain()
    await engine.resetUnsealed()
  }

  package func revokeConsent() async {
    await discardUnsealed()
    try? await engine.purge()
  }

  private func scheduleDrain() {
    Task.detached(priority: .utility) { [weak self] in
      await self?.drain()
    }
  }

  private func drain() async {
    while true {
      let step = state.withLock { state -> DrainStep in
        var events = state.gestures.map(Event.gesture)
        if let observation = state.latestObservation {
          events.append(.observation(observation))
        }
        guard !events.isEmpty else {
          state.drainScheduled = false
          let waiters = state.waiters
          state.waiters.removeAll(keepingCapacity: true)
          return .finished(waiters)
        }
        state.latestObservation = nil
        state.gestures.removeAll(keepingCapacity: true)
        let coalesced = state.coalesced
        state.coalesced = 0
        return .batch(
          Batch(
            events: events.sorted {
              $0.monotonicNano < $1.monotonicNano
            },
            coalesced: coalesced
          )
        )
      }
      guard case .batch(let batch) = step else {
        if case .finished(let waiters) = step {
          for waiter in waiters { waiter.resume() }
        }
        return
      }
      if batch.coalesced > 0 {
        await engine.noteAdmissionCoalesced(batch.coalesced)
      }
      for event in batch.events {
        switch event {
        case .observation(let observation):
          await engine.ingest(observation)
        case .gesture(let gesture):
          await engine.ingest(
            gesture: gesture.gesture,
            occurredAtUnixNano: gesture.occurredAtUnixNano,
            monotonicNano: gesture.monotonicNano
          )
        }
      }
    }
  }

  private func waitForDrain() async {
    await withCheckedContinuation { continuation in
      let complete = state.withLock { state -> Bool in
        guard state.drainScheduled else { return true }
        state.waiters.append(continuation)
        return false
      }
      if complete { continuation.resume() }
    }
  }
}
