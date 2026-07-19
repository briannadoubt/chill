import Foundation

public enum ChillNetworkingError: Error, Equatable, Sendable {
  case invalidOrigin
}

/// A canonical HTTP or HTTPS origin used as an exact propagation boundary.
public struct ChillNetworkOrigin: Hashable, Sendable, CustomStringConvertible {
  public let scheme: String
  public let host: String
  public let port: Int?

  public init(_ url: URL) throws {
    guard let rawScheme = url.scheme?.lowercased(),
      rawScheme == "http" || rawScheme == "https",
      let rawHost = url.host?.lowercased(),
      !rawHost.isEmpty,
      url.user == nil,
      url.password == nil,
      url.query == nil,
      url.fragment == nil,
      url.path.isEmpty || url.path == "/"
    else {
      throw ChillNetworkingError.invalidOrigin
    }
    let normalizedPort: Int?
    switch (rawScheme, url.port) {
    case ("http", 80), ("https", 443): normalizedPort = nil
    default: normalizedPort = url.port
    }
    scheme = rawScheme
    host = rawHost
    port = normalizedPort
  }

  package init?(destination url: URL?) {
    guard let url else { return nil }
    var components = URLComponents()
    components.scheme = url.scheme
    components.host = url.host
    components.port = url.port
    guard let originURL = components.url,
      let origin = try? ChillNetworkOrigin(originURL)
    else { return nil }
    self = origin
  }

  public var description: String {
    let renderedHost = host.contains(":") ? "[\(host)]" : host
    if let port { return "\(scheme)://\(renderedHost):\(port)" }
    return "\(scheme)://\(renderedHost)"
  }
}

/// Explicit, bounded policy for cross-process trace context.
///
/// Baggage remains empty unless both an allowlist key and a value are supplied.
/// Chill never derives baggage from annotations or product/session identity.
public struct ChillNetworkPropagationPolicy: Sendable {
  public static let none = ChillNetworkPropagationPolicy(
    trustedOrigins: [],
    baggageAllowlist: [],
    baggage: [:]
  )

  public let trustedOrigins: Set<ChillNetworkOrigin>
  public let baggageAllowlist: Set<String>
  public let baggage: [String: String]

  public init(
    trustedOrigins: Set<ChillNetworkOrigin>,
    baggageAllowlist: Set<String> = [],
    baggage: [String: String] = [:]
  ) {
    self.trustedOrigins = trustedOrigins
    self.baggageAllowlist = baggageAllowlist
    self.baggage = baggage
  }

  public init(
    trustedOriginURLs: [URL],
    baggageAllowlist: Set<String> = [],
    baggage: [String: String] = [:]
  ) throws {
    self.init(
      trustedOrigins: try Set(trustedOriginURLs.map(ChillNetworkOrigin.init)),
      baggageAllowlist: baggageAllowlist,
      baggage: baggage
    )
  }

  package func trusts(_ url: URL?) -> Bool {
    guard let origin = ChillNetworkOrigin(destination: url) else {
      return false
    }
    return trustedOrigins.contains(origin)
  }

  package var baggageHeader: String? {
    var accepted: [String] = []
    var encodedSize = 0
    for key in baggage.keys.sorted() {
      guard accepted.count < 8,
        baggageAllowlist.contains(key),
        Self.isValidBaggageKey(key),
        let value = baggage[key],
        value.utf8.count <= 128
      else { continue }
      let encodedValue = Self.percentEncodeBaggageValue(value)
      let entry = "\(key)=\(encodedValue)"
      let separatorSize = accepted.isEmpty ? 0 : 1
      guard encodedSize + separatorSize + entry.utf8.count <= 1_024 else {
        continue
      }
      accepted.append(entry)
      encodedSize += separatorSize + entry.utf8.count
    }
    return accepted.isEmpty ? nil : accepted.joined(separator: ",")
  }

  private static func isValidBaggageKey(_ key: String) -> Bool {
    !key.isEmpty
      && key.utf8.allSatisfy { byte in
        (48...57).contains(byte)
          || (65...90).contains(byte)
          || (97...122).contains(byte)
          || [33, 35, 36, 37, 38, 39, 42, 43, 45, 46, 94, 95, 96, 124, 126]
            .contains(byte)
      }
  }

  private static func percentEncodeBaggageValue(_ value: String) -> String {
    let hexadecimal = Array("0123456789ABCDEF".utf8)
    var result: [UInt8] = []
    result.reserveCapacity(value.utf8.count)
    for byte in value.utf8 {
      if isBaggageSafe(byte) {
        result.append(byte)
      } else {
        result.append(37)
        result.append(hexadecimal[Int(byte >> 4)])
        result.append(hexadecimal[Int(byte & 0x0F)])
      }
    }
    return String(decoding: result, as: UTF8.self)
  }

  private static func isBaggageSafe(_ byte: UInt8) -> Bool {
    (48...57).contains(byte)
      || (65...90).contains(byte)
      || (97...122).contains(byte)
      || [
        33, 35, 36, 38, 39, 40, 41, 42, 43, 45, 46, 47, 58, 60,
        61, 62, 63, 64, 91, 93, 94, 95, 96, 123, 124, 125, 126,
      ]
      .contains(byte)
  }
}
