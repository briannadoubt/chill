import Dispatch
import Foundation

public struct TimePoint: Equatable, Sendable {
  public let occurredAtUnixNano: UInt64
  public let monotonicNano: UInt64

  public init(occurredAtUnixNano: UInt64, monotonicNano: UInt64) {
    self.occurredAtUnixNano = occurredAtUnixNano
    self.monotonicNano = monotonicNano
  }
}

public protocol ChillClock: Sendable {
  func now() -> TimePoint
}

public struct SystemChillClock: ChillClock {
  public init() {}

  public func now() -> TimePoint {
    let seconds = Date().timeIntervalSince1970
    return TimePoint(
      occurredAtUnixNano: UInt64(max(0, seconds * 1_000_000_000)),
      monotonicNano: DispatchTime.now().uptimeNanoseconds
    )
  }
}

public struct RecordClock: Equatable, Sendable {
  public let occurredAtUnixNano: UInt64
  public let observedAtUnixNano: UInt64
  public let monotonicNano: UInt64
  public let bootID: BootID
  public let sequenceNumber: UInt64
}
