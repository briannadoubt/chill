import Foundation
import os

/// The immutable correlation attached to a canonical behavior fact.
///
/// Chill creates this value or receives it from its OpenTelemetry bridge.
/// Application code can inspect correlation, but cannot manufacture or mutate
/// identifiers through the ordinary public API.
public struct TraceContext: Equatable, Sendable {
  public let traceID: String
  public let spanID: String
  public let parentSpanID: String?
  public let traceFlags: UInt8
  public let traceState: String?
  public let isRemote: Bool

  @_spi(TraceIntegration)
  public init(
    traceID: String,
    spanID: String,
    parentSpanID: String? = nil,
    traceFlags: UInt8 = 0,
    traceState: String? = nil,
    isRemote: Bool = false
  ) throws {
    guard Self.isValidHexID(traceID, length: 32) else {
      throw TraceContractError.invalidTraceID
    }
    guard Self.isValidHexID(spanID, length: 16) else {
      throw TraceContractError.invalidSpanID
    }
    if let parentSpanID,
      !Self.isValidHexID(parentSpanID, length: 16)
    {
      throw TraceContractError.invalidParentSpanID
    }
    if let traceState {
      try Self.validateTraceState(traceState)
    }
    self.traceID = traceID
    self.spanID = spanID
    self.parentSpanID = parentSpanID
    self.traceFlags = traceFlags
    self.traceState = traceState
    self.isRemote = isRemote
  }

  package static func localChild(
    of parent: TraceContext?,
    sampled: Bool = true
  ) -> TraceContext {
    try! TraceContext(
      traceID: parent?.traceID ?? randomHexID(byteCount: 16),
      spanID: randomHexID(byteCount: 8),
      parentSpanID: parent?.spanID,
      traceFlags: parent?.traceFlags ?? (sampled ? 1 : 0),
      traceState: parent?.traceState
    )
  }

  package static func remote(
    traceID: String,
    spanID: String,
    traceFlags: UInt8,
    traceState: String?
  ) throws -> TraceContext {
    try TraceContext(
      traceID: traceID,
      spanID: spanID,
      traceFlags: traceFlags,
      traceState: traceState,
      isRemote: true
    )
  }

  private static func randomHexID(byteCount: Int) -> String {
    var generator = SystemRandomNumberGenerator()
    while true {
      let bytes = (0..<byteCount).map { _ in UInt8.random(in: .min ... .max, using: &generator) }
      if bytes.contains(where: { $0 != 0 }) {
        return bytes.map { String(format: "%02x", $0) }.joined()
      }
    }
  }

  private static func isValidHexID(_ value: String, length: Int) -> Bool {
    guard value.utf8.count == length,
      value.utf8.allSatisfy({ byte in
        (48...57).contains(byte) || (97...102).contains(byte)
      })
    else {
      return false
    }
    return value.utf8.contains(where: { $0 != 48 })
  }

  private static func validateTraceState(_ value: String) throws {
    guard let bytes = value.data(using: .ascii),
      !bytes.isEmpty,
      bytes.count <= 512
    else {
      throw TraceContractError.invalidTraceState
    }
    let members = value.split(separator: ",", omittingEmptySubsequences: false)
    guard members.count <= 32 else {
      throw TraceContractError.invalidTraceState
    }
    var keys = Set<Substring>()
    for member in members {
      let pieces = member.split(
        separator: "=",
        omittingEmptySubsequences: false
      )
      guard pieces.count == 2,
        isValidTraceStateKey(pieces[0]),
        keys.insert(pieces[0]).inserted,
        isValidTraceStateValue(pieces[1])
      else {
        throw TraceContractError.invalidTraceState
      }
    }
  }

  private static func isValidTraceStateKey(_ key: Substring) -> Bool {
    let parts = key.split(separator: "@", omittingEmptySubsequences: false)
    if parts.count == 1 {
      guard key.count <= 256,
        let first = key.utf8.first,
        (97...122).contains(first)
      else { return false }
      return key.utf8.dropFirst().allSatisfy(isTraceStateKeyByte)
    }
    guard parts.count == 2,
      !parts[0].isEmpty,
      parts[0].count <= 241,
      !parts[1].isEmpty,
      parts[1].count <= 14,
      let systemFirst = parts[0].utf8.first,
      isLowercaseAlphaNumeric(systemFirst),
      parts[0].utf8.dropFirst().allSatisfy(isTraceStateKeyByte),
      let tenantFirst = parts[1].utf8.first,
      (97...122).contains(tenantFirst),
      parts[1].utf8.dropFirst().allSatisfy(isTraceStateKeyByte)
    else { return false }
    return true
  }

  private static func isTraceStateKeyByte(_ byte: UInt8) -> Bool {
    isLowercaseAlphaNumeric(byte)
      || byte == 95 || byte == 45 || byte == 42 || byte == 47
  }

  private static func isLowercaseAlphaNumeric(_ byte: UInt8) -> Bool {
    (97...122).contains(byte) || (48...57).contains(byte)
  }

  private static func isValidTraceStateValue(_ value: Substring) -> Bool {
    guard !value.isEmpty,
      value.utf8.count <= 256,
      value.utf8.last != 32
    else { return false }
    return value.utf8.allSatisfy { byte in
      (0x20...0x2B).contains(byte)
        || (0x2D...0x3C).contains(byte)
        || (0x3E...0x7E).contains(byte)
    }
  }
}

public enum TraceContractError: Error, Equatable, Sendable {
  case invalidTraceID
  case invalidSpanID
  case invalidParentSpanID
  case invalidTraceState
}

package enum W3CTraceContext {
  package static func traceParent(for context: TraceContext) -> String {
    String(
      format: "00-%@-%@-%02x",
      context.traceID,
      context.spanID,
      context.traceFlags & 0x01
    )
  }

  package static func extract(
    traceParent: String?,
    traceState: String? = nil
  ) -> TraceContext? {
    guard let traceParent else { return nil }
    let parts = traceParent.split(
      separator: "-",
      omittingEmptySubsequences: false
    )
    guard parts.count == 4,
      parts[0] == "00",
      parts[3].utf8.allSatisfy({ byte in
        (48...57).contains(byte) || (97...102).contains(byte)
      }),
      let flags = UInt8(parts[3], radix: 16),
      parts[3].count == 2
    else { return nil }
    return try? TraceContext.remote(
      traceID: String(parts[1]),
      spanID: String(parts[2]),
      traceFlags: flags,
      traceState: traceState
    )
  }
}

package enum TraceTaskContext {
  @TaskLocal package static var current: TraceContext?
}

@_spi(TraceIntegration)
public enum ChillTraceSpanKind: Sendable {
  case `internal`
  case client
  case server
  case producer
  case consumer
}

@_spi(TraceIntegration)
public enum ChillTraceAttributeValue: Equatable, Sendable {
  case string(String)
  case integer(Int64)
  case double(Double)
  case boolean(Bool)
}

@_spi(TraceIntegration)
public struct ChillTraceSpanRequest: Sendable {
  public let name: String
  public let kind: ChillTraceSpanKind
  public let parent: TraceContext?
  public let attributes: [String: ChillTraceAttributeValue]

  package init(
    name: String,
    kind: ChillTraceSpanKind,
    parent: TraceContext?,
    attributes: [String: ChillTraceAttributeValue]
  ) {
    self.name = name
    self.kind = kind
    self.parent = parent
    self.attributes = attributes
  }
}

@_spi(TraceIntegration)
public enum ChillTraceSpanStatus: Sendable {
  case unset
  case ok
  case error
}

@_spi(TraceIntegration)
public protocol ChillTraceSpanHandle: Sendable {
  var context: TraceContext { get }
  func setAttributes(_ attributes: [String: ChillTraceAttributeValue])
  func end(status: ChillTraceSpanStatus)
}

@_spi(TraceIntegration)
public protocol ChillTraceProvider: Sendable {
  func startSpan(_ request: ChillTraceSpanRequest) -> any ChillTraceSpanHandle
}

package final class ChillTraceSpan: Sendable {
  package let context: TraceContext
  private let handle: (any ChillTraceSpanHandle)?
  private let ended = OSAllocatedUnfairLock(initialState: false)

  init(context: TraceContext, handle: (any ChillTraceSpanHandle)? = nil) {
    self.context = context
    self.handle = handle
  }

  package func end(
    status: ChillTraceSpanStatus,
    attributes: [String: ChillTraceAttributeValue] = [:]
  ) {
    let shouldEnd = ended.withLock { ended in
      guard !ended else { return false }
      ended = true
      return true
    }
    guard shouldEnd else { return }
    if !attributes.isEmpty { handle?.setAttributes(attributes) }
    handle?.end(status: status)
  }
}

package enum ChillTraceRuntime {
  private static let provider = OSAllocatedUnfairLock<
    (any ChillTraceProvider)?
  >(initialState: nil)

  @_spi(TraceIntegration)
  public static func configure(provider newProvider: any ChillTraceProvider) {
    provider.withLock { $0 = newProvider }
  }

  @_spi(TraceIntegration)
  public static func resetProvider() {
    provider.withLock { $0 = nil }
  }

  package static var isProviderConfigured: Bool {
    provider.withLock { $0 != nil }
  }

  package static func startSpan(
    name: String,
    kind: ChillTraceSpanKind,
    attributes: [String: ChillTraceAttributeValue] = [:]
  ) -> ChillTraceSpan {
    let request = ChillTraceSpanRequest(
      name: name,
      kind: kind,
      parent: TraceTaskContext.current,
      attributes: attributes
    )
    if let provider = provider.withLock({ $0 }) {
      let handle = provider.startSpan(request)
      return ChillTraceSpan(context: handle.context, handle: handle)
    }
    return ChillTraceSpan(
      context: .localChild(of: request.parent)
    )
  }
}

extension Chill {
  /// Installs Chill's adapter to an application's existing OpenTelemetry
  /// provider. This SPI is intended for official integration packages; Chill
  /// never installs or replaces a process-global provider itself.
  @_spi(TraceIntegration)
  public static func configureTraceProvider(
    _ provider: any ChillTraceProvider
  ) {
    ChillTraceRuntime.configure(provider: provider)
  }
}
