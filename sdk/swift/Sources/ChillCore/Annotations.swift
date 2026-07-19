import Foundation

public struct AnnotationName: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    let parts = rawValue.split(separator: ".", omittingEmptySubsequences: false)
    guard rawValue.utf8.count <= 128,
      !parts.isEmpty,
      parts.allSatisfy({ part in
        guard let first = part.first,
          first.isASCII,
          first.isLowercase
        else {
          return false
        }
        return part.dropFirst().allSatisfy {
          $0.isASCII
            && ($0.isLowercase || $0.isNumber || $0 == "_")
        }
      })
    else {
      throw ContractError.invalidAnnotationName(rawValue)
    }
    self.rawValue = rawValue
  }

  public var description: String { rawValue }
}

public enum AnnotationValue: Equatable, Codable, Sendable {
  case string(String)
  case boolean(Bool)
  case integer(Int64)
  case double(Double)
  case strings([String])
  case booleans([Bool])
  case integers([Int64])
  case doubles([Double])

  public func validate() throws {
    switch self {
    case .string(let value):
      guard value.utf8.count <= 4_096 else {
        throw ContractError.invalidAnnotationValue
      }
    case .double(let value):
      guard value.isFinite else {
        throw ContractError.invalidAnnotationValue
      }
    case .doubles(let values):
      guard values.count <= 64, values.allSatisfy(\.isFinite) else {
        throw ContractError.invalidAnnotationValue
      }
    case .strings(let values):
      guard values.count <= 64,
        values.allSatisfy({ $0.utf8.count <= 4_096 })
      else {
        throw ContractError.invalidAnnotationValue
      }
    case .booleans(let values):
      guard values.count <= 64 else {
        throw ContractError.invalidAnnotationValue
      }
    case .integers(let values):
      guard values.count <= 64 else {
        throw ContractError.invalidAnnotationValue
      }
    default:
      break
    }
  }

  public init(from decoder: any Decoder) throws {
    let container = try decoder.singleValueContainer()
    if let value = try? container.decode(String.self) {
      self = .string(value)
    } else if let value = try? container.decode(Bool.self) {
      self = .boolean(value)
    } else if let value = try? container.decode(Int64.self) {
      self = .integer(value)
    } else if let value = try? container.decode(Double.self) {
      self = .double(value)
    } else if let value = try? container.decode([String].self) {
      self = .strings(value)
    } else if let value = try? container.decode([Bool].self) {
      self = .booleans(value)
    } else if let value = try? container.decode([Int64].self) {
      self = .integers(value)
    } else if let value = try? container.decode([Double].self) {
      self = .doubles(value)
    } else {
      throw DecodingError.dataCorruptedError(
        in: container,
        debugDescription: "unsupported Chill annotation value"
      )
    }
    try validate()
  }

  public func encode(to encoder: any Encoder) throws {
    try validate()
    var container = encoder.singleValueContainer()
    switch self {
    case .string(let value):
      try container.encode(value)
    case .boolean(let value):
      try container.encode(value)
    case .integer(let value):
      try container.encode(value)
    case .double(let value):
      try container.encode(value)
    case .strings(let value):
      try container.encode(value)
    case .booleans(let value):
      try container.encode(value)
    case .integers(let value):
      try container.encode(value)
    case .doubles(let value):
      try container.encode(value)
    }
  }
}

extension AnnotationValue: AnnotationValueConvertible {
  public func toAnnotationValue() throws -> AnnotationValue {
    try validate()
    return self
  }

  public static func fromAnnotationValue(
    _ value: AnnotationValue
  ) -> AnnotationValue? {
    value
  }
}

public protocol AnnotationValueConvertible: Sendable {
  func toAnnotationValue() throws -> AnnotationValue
  static func fromAnnotationValue(_ value: AnnotationValue) -> Self?
}

public protocol AnnotationScalar: AnnotationValueConvertible {
  static func makeArrayValue(_ values: [Self]) throws -> AnnotationValue
  static func fromArrayValue(_ value: AnnotationValue) -> [Self]?
}

extension String: AnnotationScalar {
  public func toAnnotationValue() throws -> AnnotationValue {
    let value = AnnotationValue.string(self)
    try value.validate()
    return value
  }

  public static func fromAnnotationValue(_ value: AnnotationValue) -> String? {
    guard case .string(let value) = value else { return nil }
    return value
  }

  public static func makeArrayValue(_ values: [String]) throws -> AnnotationValue {
    let value = AnnotationValue.strings(values)
    try value.validate()
    return value
  }

  public static func fromArrayValue(_ value: AnnotationValue) -> [String]? {
    guard case .strings(let value) = value else { return nil }
    return value
  }
}

extension Bool: AnnotationScalar {
  public func toAnnotationValue() -> AnnotationValue { .boolean(self) }

  public static func fromAnnotationValue(_ value: AnnotationValue) -> Bool? {
    guard case .boolean(let value) = value else { return nil }
    return value
  }

  public static func makeArrayValue(_ values: [Bool]) throws -> AnnotationValue {
    let value = AnnotationValue.booleans(values)
    try value.validate()
    return value
  }

  public static func fromArrayValue(_ value: AnnotationValue) -> [Bool]? {
    guard case .booleans(let value) = value else { return nil }
    return value
  }
}

extension Int64: AnnotationScalar {
  public func toAnnotationValue() -> AnnotationValue { .integer(self) }

  public static func fromAnnotationValue(_ value: AnnotationValue) -> Int64? {
    guard case .integer(let value) = value else { return nil }
    return value
  }

  public static func makeArrayValue(_ values: [Int64]) throws -> AnnotationValue {
    let value = AnnotationValue.integers(values)
    try value.validate()
    return value
  }

  public static func fromArrayValue(_ value: AnnotationValue) -> [Int64]? {
    guard case .integers(let value) = value else { return nil }
    return value
  }
}

extension Int: AnnotationScalar {
  public func toAnnotationValue() -> AnnotationValue {
    .integer(Int64(self))
  }

  public static func fromAnnotationValue(_ value: AnnotationValue) -> Int? {
    guard case .integer(let value) = value else { return nil }
    return Int(exactly: value)
  }

  public static func makeArrayValue(_ values: [Int]) throws -> AnnotationValue {
    let value = AnnotationValue.integers(values.map(Int64.init))
    try value.validate()
    return value
  }

  public static func fromArrayValue(_ value: AnnotationValue) -> [Int]? {
    guard case .integers(let value) = value else { return nil }
    let converted = value.compactMap(Int.init(exactly:))
    return converted.count == value.count ? converted : nil
  }
}

extension Double: AnnotationScalar {
  public func toAnnotationValue() throws -> AnnotationValue {
    let value = AnnotationValue.double(self)
    try value.validate()
    return value
  }

  public static func fromAnnotationValue(_ value: AnnotationValue) -> Double? {
    guard case .double(let value) = value else { return nil }
    return value
  }

  public static func makeArrayValue(_ values: [Double]) throws -> AnnotationValue {
    let value = AnnotationValue.doubles(values)
    try value.validate()
    return value
  }

  public static func fromArrayValue(_ value: AnnotationValue) -> [Double]? {
    guard case .doubles(let value) = value else { return nil }
    return value
  }
}

extension Array: AnnotationValueConvertible where Element: AnnotationScalar {
  public func toAnnotationValue() throws -> AnnotationValue {
    try Element.makeArrayValue(self)
  }

  public static func fromAnnotationValue(_ value: AnnotationValue) -> [Element]? {
    Element.fromArrayValue(value)
  }
}

public struct AnnotationKey<Value: AnnotationValueConvertible>: Hashable,
  Sendable
{
  public let name: AnnotationName

  public init(_ name: String) throws {
    self.name = try AnnotationName(name)
  }
}

public struct AnnotationDeclaration: Equatable, Sendable {
  public let name: AnnotationName
  public let value: AnnotationValue

  public init<Value: AnnotationValueConvertible>(
    _ name: String,
    value: Value
  ) throws {
    self.name = try AnnotationName(name)
    self.value = try value.toAnnotationValue()
  }

  public init<Value: AnnotationValueConvertible>(
    _ key: AnnotationKey<Value>,
    value: Value
  ) throws {
    name = key.name
    self.value = try value.toAnnotationValue()
  }

  public init(name: AnnotationName, value: AnnotationValue) throws {
    try value.validate()
    self.name = name
    self.value = value
  }
}

public struct AnnotationOrigin: Equatable, Sendable {
  public let scopeID: AnnotationScopeID
  public let depth: Int
  public let declarationIndex: Int
}

public struct AnnotationCollision: Equatable, Sendable {
  public let name: AnnotationName
  public let winner: AnnotationOrigin
  public let loser: AnnotationOrigin
  public let identical: Bool
}

public struct AnnotationSnapshot: Equatable, Sendable {
  public static let empty = AnnotationSnapshot(
    values: [:],
    origins: [:],
    collisions: [],
    classifications: [:]
  )

  public let values: [AnnotationName: AnnotationValue]
  public let origins: [AnnotationName: AnnotationOrigin]
  public let collisions: [AnnotationCollision]
  public let classifications: [AnnotationName: DataClassification]

  package init(
    values: [AnnotationName: AnnotationValue],
    origins: [AnnotationName: AnnotationOrigin],
    collisions: [AnnotationCollision],
    classifications: [AnnotationName: DataClassification] = [:]
  ) {
    self.values = values
    self.origins = origins
    self.collisions = collisions
    self.classifications = classifications
  }

  public subscript<Value: AnnotationValueConvertible>(
    key: AnnotationKey<Value>
  ) -> Value? {
    values[key.name].flatMap(Value.fromAnnotationValue)
  }
}

public struct AnnotationContext: Equatable, Sendable {
  private final class Storage: Sendable {
    let depth: Int
    let scopeIDs: Set<AnnotationScopeID>
    let values: [AnnotationName: AnnotationValue]
    let origins: [AnnotationName: AnnotationOrigin]
    let collisions: [AnnotationCollision]

    init(
      depth: Int,
      scopeIDs: Set<AnnotationScopeID>,
      values: [AnnotationName: AnnotationValue],
      origins: [AnnotationName: AnnotationOrigin],
      collisions: [AnnotationCollision]
    ) {
      self.depth = depth
      self.scopeIDs = scopeIDs
      self.values = values
      self.origins = origins
      self.collisions = collisions
    }
  }

  private static let emptyStorage = Storage(
    depth: -1,
    scopeIDs: [],
    values: [:],
    origins: [:],
    collisions: []
  )

  private let storage: Storage

  public init() {
    storage = Self.emptyStorage
  }

  private init(storage: Storage) {
    self.storage = storage
  }

  public var snapshot: AnnotationSnapshot {
    AnnotationSnapshot(
      values: storage.values,
      origins: storage.origins,
      collisions: storage.collisions,
      classifications: [:]
    )
  }

  public func addingScope(
    id: AnnotationScopeID,
    declarations: [AnnotationDeclaration]
  ) throws -> AnnotationContext {
    guard !storage.scopeIDs.contains(id) else {
      throw ContractError.duplicateAnnotationScope(id.rawValue)
    }

    let depth = storage.depth + 1
    var values = storage.values
    var origins = storage.origins
    var collisions = storage.collisions
    for (index, declaration) in declarations.enumerated() {
      let origin = AnnotationOrigin(
        scopeID: id,
        depth: depth,
        declarationIndex: index
      )
      if let winner = origins[declaration.name],
        let winnerValue = values[declaration.name]
      {
        collisions.append(
          AnnotationCollision(
            name: declaration.name,
            winner: winner,
            loser: origin,
            identical: winnerValue == declaration.value
          )
        )
      } else {
        guard values.count < 128 else {
          throw ContractError.annotationLimitExceeded
        }
        values[declaration.name] = declaration.value
        origins[declaration.name] = origin
      }
    }
    var scopeIDs = storage.scopeIDs
    scopeIDs.insert(id)
    return AnnotationContext(
      storage: Storage(
        depth: depth,
        scopeIDs: scopeIDs,
        values: values,
        origins: origins,
        collisions: collisions
      )
    )
  }

  public subscript<Value: AnnotationValueConvertible>(
    key: AnnotationKey<Value>
  ) -> Value? {
    snapshot[key]
  }

  public static func == (left: AnnotationContext, right: AnnotationContext) -> Bool {
    left.snapshot == right.snapshot
  }
}
