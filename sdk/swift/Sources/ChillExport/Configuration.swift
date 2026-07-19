import Foundation

public enum ChillExportConfigurationError: Error, Equatable, Sendable {
  case invalidEndpoint
  case invalidHeader
  case invalidResourceAttribute
  case invalidQueueLimit
  case invalidBatchLimit
  case invalidFlushInterval
  case invalidRetryPolicy
}

/// Durable delivery priority. Capacity pressure evicts replay first, then
/// impressions; high-value lifecycle and action facts are retained last.
public enum ChillQueuePriority: UInt8, CaseIterable, Sendable {
  case replay = 0
  case low = 1
  case high = 2
}

public struct ChillRetryConfiguration: Equatable, Sendable {
  public static let standard = ChillRetryConfiguration(
    initialDelay: 1,
    maximumDelay: 60,
    multiplier: 2,
    jitterRatio: 0.2,
    validated: ()
  )

  public let initialDelay: TimeInterval
  public let maximumDelay: TimeInterval
  public let multiplier: Double
  public let jitterRatio: Double

  public init(
    initialDelay: TimeInterval = 1,
    maximumDelay: TimeInterval = 60,
    multiplier: Double = 2,
    jitterRatio: Double = 0.2
  ) throws {
    guard initialDelay.isFinite,
      maximumDelay.isFinite,
      multiplier.isFinite,
      jitterRatio.isFinite,
      initialDelay >= 0.01,
      maximumDelay >= initialDelay,
      maximumDelay <= 3_600,
      multiplier >= 1,
      multiplier <= 10,
      (0...1).contains(jitterRatio)
    else {
      throw ChillExportConfigurationError.invalidRetryPolicy
    }
    self.init(
      initialDelay: initialDelay,
      maximumDelay: maximumDelay,
      multiplier: multiplier,
      jitterRatio: jitterRatio,
      validated: ()
    )
  }

  private init(
    initialDelay: TimeInterval,
    maximumDelay: TimeInterval,
    multiplier: Double,
    jitterRatio: Double,
    validated: Void
  ) {
    self.initialDelay = initialDelay
    self.maximumDelay = maximumDelay
    self.multiplier = multiplier
    self.jitterRatio = jitterRatio
  }

  package func delay(attempt: Int, unitRandom: Double) -> TimeInterval {
    let exponent = Double(max(0, min(attempt, 30)))
    let base = min(maximumDelay, initialDelay * pow(multiplier, exponent))
    let boundedRandom = min(1, max(0, unitRandom))
    let jitter = (boundedRandom * 2 - 1) * jitterRatio
    return min(maximumDelay, max(0.01, base * (1 + jitter)))
  }
}

public struct ChillOfflineConfiguration: Equatable, Sendable {
  public static let defaultDiskLimit = 64 * 1_024 * 1_024
  public static let standard = ChillOfflineConfiguration(
    maximumDiskBytes: defaultDiskLimit,
    maximumMemoryRecords: 1_024,
    maximumBatchRecords: 100,
    maximumBatchBytes: 512 * 1_024,
    flushInterval: 10,
    retry: .standard,
    validated: ()
  )

  public let maximumDiskBytes: Int
  public let maximumMemoryRecords: Int
  public let maximumBatchRecords: Int
  public let maximumBatchBytes: Int
  public let flushInterval: TimeInterval
  public let retry: ChillRetryConfiguration

  public init(
    maximumDiskBytes: Int = Self.defaultDiskLimit,
    maximumMemoryRecords: Int = 1_024,
    maximumBatchRecords: Int = 100,
    maximumBatchBytes: Int = 512 * 1_024,
    flushInterval: TimeInterval = 10,
    retry: ChillRetryConfiguration = .standard
  ) throws {
    guard (64 * 1_024...Self.defaultDiskLimit).contains(maximumDiskBytes),
      (16...8_192).contains(maximumMemoryRecords)
    else {
      throw ChillExportConfigurationError.invalidQueueLimit
    }
    guard (1...1_000).contains(maximumBatchRecords),
      (16 * 1_024...4 * 1_024 * 1_024).contains(maximumBatchBytes)
    else {
      throw ChillExportConfigurationError.invalidBatchLimit
    }
    guard flushInterval.isFinite,
      (0.01...300).contains(flushInterval)
    else {
      throw ChillExportConfigurationError.invalidFlushInterval
    }
    self.init(
      maximumDiskBytes: maximumDiskBytes,
      maximumMemoryRecords: maximumMemoryRecords,
      maximumBatchRecords: maximumBatchRecords,
      maximumBatchBytes: maximumBatchBytes,
      flushInterval: flushInterval,
      retry: retry,
      validated: ()
    )
  }

  private init(
    maximumDiskBytes: Int,
    maximumMemoryRecords: Int,
    maximumBatchRecords: Int,
    maximumBatchBytes: Int,
    flushInterval: TimeInterval,
    retry: ChillRetryConfiguration,
    validated: Void
  ) {
    self.maximumDiskBytes = maximumDiskBytes
    self.maximumMemoryRecords = maximumMemoryRecords
    self.maximumBatchRecords = maximumBatchRecords
    self.maximumBatchBytes = maximumBatchBytes
    self.flushInterval = flushInterval
    self.retry = retry
  }
}

/// Configuration for the dedicated OTLP/HTTP Logs exporter. Authentication
/// headers are held only in memory and are never persisted or included in
/// diagnostics.
public struct ChillOTLPConfiguration: Sendable {
  public let endpoint: URL
  public let headers: [String: String]
  public let resourceAttributes: [String: String]
  public let offline: ChillOfflineConfiguration

  public init(
    endpoint: URL,
    headers: [String: String] = [:],
    resourceAttributes: [String: String] = [:],
    offline: ChillOfflineConfiguration = .standard,
    allowInsecureLocalhost: Bool = false
  ) throws {
    guard Self.isAllowed(endpoint, allowInsecureLocalhost: allowInsecureLocalhost)
    else {
      throw ChillExportConfigurationError.invalidEndpoint
    }
    guard headers.count <= 32,
      headers.allSatisfy({ key, value in
        Self.isToken(key)
          && key.utf8.count <= 128
          && value.utf8.count <= 4_096
          && !value.utf8.contains(13)
          && !value.utf8.contains(10)
          && !["content-length", "content-encoding", "content-type"]
            .contains(key.lowercased())
      })
    else {
      throw ChillExportConfigurationError.invalidHeader
    }
    guard resourceAttributes.count <= 64,
      resourceAttributes.allSatisfy({ key, value in
        !key.isEmpty
          && key.utf8.count <= 128
          && key.utf8.allSatisfy({ byte in
            (48...57).contains(byte)
              || (65...90).contains(byte)
              || (97...122).contains(byte)
              || byte == 46 || byte == 95 || byte == 45
          })
          && value.utf8.count <= 1_024
      })
    else {
      throw ChillExportConfigurationError.invalidResourceAttribute
    }
    self.endpoint = endpoint
    self.headers = headers
    self.resourceAttributes = resourceAttributes
    self.offline = offline
  }

  private static func isAllowed(
    _ endpoint: URL,
    allowInsecureLocalhost: Bool
  ) -> Bool {
    guard endpoint.user == nil,
      endpoint.password == nil,
      endpoint.query == nil,
      endpoint.fragment == nil,
      endpoint.host != nil
    else { return false }
    if endpoint.scheme?.lowercased() == "https" { return true }
    guard allowInsecureLocalhost,
      endpoint.scheme?.lowercased() == "http",
      let host = endpoint.host?.lowercased()
    else { return false }
    return host == "localhost" || host == "127.0.0.1" || host == "::1"
  }

  private static func isToken(_ value: String) -> Bool {
    !value.isEmpty
      && value.utf8.allSatisfy { byte in
        (48...57).contains(byte)
          || (65...90).contains(byte)
          || (97...122).contains(byte)
          || [33, 35, 36, 37, 38, 39, 42, 43, 45, 46, 94, 95, 96, 124, 126]
            .contains(byte)
      }
  }
}

public struct ChillExporterMetrics: Equatable, Sendable {
  public let queuedRecords: Int
  public let queuedBytes: Int
  public let memoryPendingRecords: Int
  public let recoveredRecords: Int
  public let quarantinedFiles: Int
  public let persistedRecords: UInt64
  public let acknowledgedRecords: UInt64
  public let duplicateRecords: UInt64
  public let droppedReplayRecords: UInt64
  public let droppedLowPriorityRecords: UInt64
  public let droppedHighPriorityRecords: UInt64
  public let exportAttempts: UInt64
  public let exportFailures: UInt64
  public let consecutiveFailures: UInt32
}

public enum ChillFlushResult: Equatable, Sendable {
  case emptied
  case retryScheduled
  case blocked
}
