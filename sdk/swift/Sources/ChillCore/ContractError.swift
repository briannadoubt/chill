public enum ContractError: Error, Equatable, Sendable {
  case emptyIdentifier(type: String)
  case invalidAnnotationName(String)
  case invalidSemanticName(String)
  case invalidPageSegment(String)
  case invalidAnnotationValue
  case annotationLimitExceeded
  case duplicateAnnotationScope(String)
  case pagePathLengthMismatch
  case emptyPagePath
  case pagePathLimitExceeded
  case invalidSamplingRate
  case invalidPrivacyPolicyVersion
  case invalidCollectionState
  case invalidBehaviorOperation
  case payloadKindMismatch
  case invalidVisibilityRatio
  case invalidReplayTimeRange
  case monotonicTimeMovedBackwards
  case invalidLifecycleTransition
}

public protocol StringBackedContractValue: Codable, Sendable {
  var rawValue: String { get }
  init(_ rawValue: String) throws
}

extension StringBackedContractValue {
  public init(from decoder: any Decoder) throws {
    let container = try decoder.singleValueContainer()
    try self.init(container.decode(String.self))
  }

  public func encode(to encoder: any Encoder) throws {
    var container = encoder.singleValueContainer()
    try container.encode(rawValue)
  }
}
