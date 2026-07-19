public struct RemoteCollectionState: Codable, Equatable, Sendable {
  public let revision: UInt64
  public let policyVersion: String
  public let enabled: Bool
  public let disabledCaptureClasses: [CaptureClass]
  public let effectiveAtUnixNano: UInt64

  public init(
    revision: UInt64,
    policyVersion: String,
    enabled: Bool,
    disabledCaptureClasses: [CaptureClass],
    effectiveAtUnixNano: UInt64
  ) throws {
    guard revision > 0, effectiveAtUnixNano > 0 else {
      throw ContractError.invalidCollectionState
    }
    self.policyVersion = try _validatedIdentifier(
      policyVersion,
      type: "RemoteCollectionState.policyVersion"
    )
    let unique = Set(disabledCaptureClasses)
    guard unique.count == disabledCaptureClasses.count else {
      throw ContractError.invalidCollectionState
    }
    self.revision = revision
    self.enabled = enabled
    self.disabledCaptureClasses = unique.sorted { $0.rawValue < $1.rawValue }
    self.effectiveAtUnixNano = effectiveAtUnixNano
  }

  private enum CodingKeys: String, CodingKey {
    case revision
    case policyVersion = "policy_version"
    case enabled
    case disabledCaptureClasses = "disabled_capture_classes"
    case effectiveAtUnixNano = "effective_at_unix_nano"
  }

  public init(from decoder: any Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    try self.init(
      revision: container.decode(UInt64.self, forKey: .revision),
      policyVersion: container.decode(String.self, forKey: .policyVersion),
      enabled: container.decode(Bool.self, forKey: .enabled),
      disabledCaptureClasses: container.decode(
        [CaptureClass].self,
        forKey: .disabledCaptureClasses
      ),
      effectiveAtUnixNano: container.decode(
        UInt64.self,
        forKey: .effectiveAtUnixNano
      )
    )
  }
}

public enum CollectionStateApplyResult: Equatable, Sendable {
  case applied
  case stale
}
