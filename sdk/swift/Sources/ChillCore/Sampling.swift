import CryptoKit
import Foundation

public enum SamplingStream: String, Codable, Sendable {
  case behavior
  case replay
}

public struct SamplingRate: Equatable, Sendable {
  public let numerator: UInt64
  public let denominator: UInt64

  public init(numerator: UInt64, denominator: UInt64) throws {
    guard denominator > 0, numerator <= denominator else {
      throw ContractError.invalidSamplingRate
    }
    self.numerator = numerator
    self.denominator = denominator
  }

  public static let none = try! SamplingRate(numerator: 0, denominator: 1)
  public static let all = try! SamplingRate(numerator: 1, denominator: 1)
}

public struct SamplingDecision: Equatable, Sendable {
  public let kept: Bool
  public let digestPrefix: String
}

public struct DeterministicSampler: Sendable {
  private let salt: String

  public init(salt: String) throws {
    self.salt = try _validatedIdentifier(salt, type: "sampling salt")
  }

  public func decide(
    stream: SamplingStream,
    stableKey: String,
    rate: SamplingRate
  ) -> SamplingDecision {
    let material = "chill-sampling-v1\0\(stream.rawValue)\0\(salt)\0\(stableKey)"
    let digest = SHA256.hash(data: Data(material.utf8))
    var value: UInt64 = 0
    var prefix = ""
    for byte in digest.prefix(8) {
      value = (value << 8) | UInt64(byte)
      prefix.append(String(format: "%02x", byte))
    }
    let kept: Bool
    if rate.numerator == 0 {
      kept = false
    } else if rate.numerator == rate.denominator {
      kept = true
    } else {
      let threshold = rate.denominator.dividingFullWidth(
        (high: rate.numerator, low: 0)
      ).quotient
      kept = value < threshold
    }
    return SamplingDecision(
      kept: kept,
      digestPrefix: prefix
    )
  }
}

public struct SamplingConfiguration: Equatable, Sendable {
  public let stableKey: String
  public let salt: String
  public let behavior: SamplingRate
  public let replay: SamplingRate

  public init(
    stableKey: String,
    salt: String,
    behavior: SamplingRate = .all,
    replay: SamplingRate = .none
  ) throws {
    self.stableKey = try _validatedIdentifier(
      stableKey,
      type: "sampling stable key"
    )
    self.salt = try _validatedIdentifier(salt, type: "sampling salt")
    self.behavior = behavior
    self.replay = replay
  }
}
