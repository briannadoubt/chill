import Foundation

@usableFromInline
func _validatedIdentifier(_ value: String, type: String) throws -> String {
  guard !value.isEmpty else {
    throw ContractError.emptyIdentifier(type: type)
  }
  return value
}

public struct RecordID: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    self.rawValue = try _validatedIdentifier(rawValue, type: "RecordID")
  }

  public var description: String { rawValue }
}

public struct SubjectID: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    self.rawValue = try _validatedIdentifier(rawValue, type: "SubjectID")
  }

  public var description: String { rawValue }
}

public struct SessionID: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    self.rawValue = try _validatedIdentifier(rawValue, type: "SessionID")
  }

  public var description: String { rawValue }
}

public struct SurfaceID: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    self.rawValue = try _validatedIdentifier(rawValue, type: "SurfaceID")
  }

  public var description: String { rawValue }
}

public struct PageInstanceID: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    self.rawValue = try _validatedIdentifier(rawValue, type: "PageInstanceID")
  }

  public var description: String { rawValue }
}

public struct ElementInstanceID: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    self.rawValue = try _validatedIdentifier(
      rawValue,
      type: "ElementInstanceID"
    )
  }

  public var description: String { rawValue }
}

public struct BootID: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    self.rawValue = try _validatedIdentifier(rawValue, type: "BootID")
  }

  public var description: String { rawValue }
}

public struct AnnotationScopeID: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    self.rawValue = try _validatedIdentifier(
      rawValue,
      type: "AnnotationScopeID"
    )
  }

  public var description: String { rawValue }
}

public struct SemanticName: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    guard rawValue.utf8.count <= 128,
      _isSemanticToken(rawValue, separators: [".", "-"])
    else {
      throw ContractError.invalidSemanticName(rawValue)
    }
    self.rawValue = rawValue
  }

  public var description: String { rawValue }
}

public struct PageSegment: Hashable, StringBackedContractValue,
  CustomStringConvertible
{
  public let rawValue: String

  public init(_ rawValue: String) throws {
    guard rawValue.utf8.count <= 80,
      _isSemanticToken(rawValue, separators: [".", "-"])
    else {
      throw ContractError.invalidPageSegment(rawValue)
    }
    self.rawValue = rawValue
  }

  public var description: String { rawValue }
}

@usableFromInline
func _isSemanticToken(_ value: String, separators: Set<Character>) -> Bool {
  guard let first = value.first, first.isASCII, first.isLowercase else {
    return false
  }
  var previousWasSeparator = false
  for character in value.dropFirst() {
    if separators.contains(character) {
      if previousWasSeparator {
        return false
      }
      previousWasSeparator = true
      continue
    }
    guard character.isASCII,
      character.isLowercase || character.isNumber || character == "_"
    else {
      return false
    }
    previousWasSeparator = false
  }
  return !previousWasSeparator
}

package protocol RecordIDGenerating: Sendable {
  func nextRecordID() -> RecordID
}

package struct SystemRecordIDGenerator: RecordIDGenerating {
  package init() {}

  package func nextRecordID() -> RecordID {
    try! RecordID(UUIDv7.generate().uuidString.lowercased())
  }
}

public enum UUIDv7 {
  public static func generate() -> UUID {
    var bytes = (0..<16).map { _ in UInt8.random(in: .min ... .max) }
    let milliseconds = UInt64(Date().timeIntervalSince1970 * 1_000)
    bytes[0] = UInt8(truncatingIfNeeded: milliseconds >> 40)
    bytes[1] = UInt8(truncatingIfNeeded: milliseconds >> 32)
    bytes[2] = UInt8(truncatingIfNeeded: milliseconds >> 24)
    bytes[3] = UInt8(truncatingIfNeeded: milliseconds >> 16)
    bytes[4] = UInt8(truncatingIfNeeded: milliseconds >> 8)
    bytes[5] = UInt8(truncatingIfNeeded: milliseconds)
    bytes[6] = (bytes[6] & 0x0f) | 0x70
    bytes[8] = (bytes[8] & 0x3f) | 0x80
    return UUID(
      uuid: (
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11],
        bytes[12], bytes[13], bytes[14], bytes[15]
      )
    )
  }
}
