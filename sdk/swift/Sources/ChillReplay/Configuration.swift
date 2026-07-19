import Foundation
import Security

public enum ChillReplayError: Error, Equatable, Sendable {
  case alreadyConfigured
  case invalidEncryptionKey
  case invalidConfiguration
  case keychainFailure(Int32)
  case chunkNotFound
  case chunkTooLarge
  case corruptChunk
}

/// A 256-bit replay key. Persistent keys live in the system Keychain; replay
/// chunk files contain only ciphertext.
public struct ChillReplayEncryptionKey: Sendable {
  package let bytes: Data

  public init(data: Data) throws {
    guard data.count == 32 else { throw ChillReplayError.invalidEncryptionKey }
    bytes = data
  }

  /// Loads or creates a device-local key protected by the system Keychain.
  public static func persistent(
    identifier: String = "default"
  ) throws -> ChillReplayEncryptionKey {
    guard !identifier.isEmpty, identifier.utf8.count <= 128 else {
      throw ChillReplayError.invalidEncryptionKey
    }
    let service = "dev.chill.session-replay"
    let base: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: identifier,
    ]
    var lookup = base
    lookup[kSecReturnData as String] = true
    lookup[kSecMatchLimit as String] = kSecMatchLimitOne
    var value: CFTypeRef?
    let lookupStatus = SecItemCopyMatching(lookup as CFDictionary, &value)
    if lookupStatus == errSecSuccess, let data = value as? Data {
      return try ChillReplayEncryptionKey(data: data)
    }
    guard lookupStatus == errSecItemNotFound else {
      throw ChillReplayError.keychainFailure(lookupStatus)
    }

    var data = Data(count: 32)
    let randomStatus = data.withUnsafeMutableBytes { buffer in
      SecRandomCopyBytes(kSecRandomDefault, 32, buffer.baseAddress!)
    }
    guard randomStatus == errSecSuccess else {
      throw ChillReplayError.keychainFailure(randomStatus)
    }
    var insertion = base
    insertion[kSecValueData as String] = data
    insertion[kSecAttrAccessible as String] =
      kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    let insertionStatus = SecItemAdd(insertion as CFDictionary, nil)
    if insertionStatus == errSecDuplicateItem {
      value = nil
      let retryStatus = SecItemCopyMatching(lookup as CFDictionary, &value)
      guard retryStatus == errSecSuccess, let existing = value as? Data else {
        throw ChillReplayError.keychainFailure(retryStatus)
      }
      return try ChillReplayEncryptionKey(data: existing)
    }
    guard insertionStatus == errSecSuccess else {
      throw ChillReplayError.keychainFailure(insertionStatus)
    }
    return try ChillReplayEncryptionKey(data: data)
  }
}

public struct ChillReplayConfiguration: Sendable {
  public let directory: URL
  public let encryptionKey: ChillReplayEncryptionKey
  public let captureInterval: TimeInterval
  public let chunkDuration: TimeInterval
  public let maximumNodes: Int
  public let maximumFramesPerChunk: Int
  public let maximumMemoryBytes: Int
  public let maximumDiskBytes: Int
  public let maximumChunkBytes: Int

  public init(
    directory: URL,
    encryptionKey: ChillReplayEncryptionKey,
    captureInterval: TimeInterval = 0.25,
    chunkDuration: TimeInterval = 10,
    maximumNodes: Int = 4_096,
    maximumFramesPerChunk: Int = 512,
    maximumMemoryBytes: Int = 8 * 1_024 * 1_024,
    maximumDiskBytes: Int = 64 * 1_024 * 1_024,
    maximumChunkBytes: Int = 1 * 1_024 * 1_024
  ) throws {
    guard directory.isFileURL,
      captureInterval.isFinite,
      (0.05...5).contains(captureInterval),
      chunkDuration.isFinite,
      (1...60).contains(chunkDuration),
      (1...16_384).contains(maximumNodes),
      (2...4_096).contains(maximumFramesPerChunk),
      (64 * 1_024...16 * 1_024 * 1_024).contains(maximumMemoryBytes),
      (1 * 1_024 * 1_024...256 * 1_024 * 1_024).contains(maximumDiskBytes),
      (32 * 1_024...4 * 1_024 * 1_024).contains(maximumChunkBytes),
      maximumChunkBytes <= maximumMemoryBytes,
      maximumChunkBytes <= maximumDiskBytes
    else {
      throw ChillReplayError.invalidConfiguration
    }
    self.directory = directory
    self.encryptionKey = encryptionKey
    self.captureInterval = captureInterval
    self.chunkDuration = chunkDuration
    self.maximumNodes = maximumNodes
    self.maximumFramesPerChunk = maximumFramesPerChunk
    self.maximumMemoryBytes = maximumMemoryBytes
    self.maximumDiskBytes = maximumDiskBytes
    self.maximumChunkBytes = maximumChunkBytes
  }

  public static func standard(
    directory: URL,
    keyIdentifier: String = "default"
  ) throws -> ChillReplayConfiguration {
    try ChillReplayConfiguration(
      directory: directory,
      encryptionKey: .persistent(identifier: keyIdentifier)
    )
  }
}

public struct ChillReplayChunkDescriptor: Equatable, Sendable {
  public let replayID: String
  public let chunkID: String
  public let chunkIndex: UInt32
  public let sessionID: String
  public let bootID: String
  public let startMonotonicNano: UInt64
  public let endMonotonicNano: UInt64
  public let occurredAtUnixNano: UInt64
  public let digest: String
  public let byteCount: Int

  package init(
    replayID: String,
    chunkID: String,
    chunkIndex: UInt32,
    sessionID: String,
    bootID: String,
    startMonotonicNano: UInt64,
    endMonotonicNano: UInt64,
    occurredAtUnixNano: UInt64,
    digest: String,
    byteCount: Int
  ) {
    self.replayID = replayID
    self.chunkID = chunkID
    self.chunkIndex = chunkIndex
    self.sessionID = sessionID
    self.bootID = bootID
    self.startMonotonicNano = startMonotonicNano
    self.endMonotonicNano = endMonotonicNano
    self.occurredAtUnixNano = occurredAtUnixNano
    self.digest = digest
    self.byteCount = byteCount
  }
}

public struct ChillReplayMetrics: Equatable, Sendable {
  public let pendingChunks: Int
  public let pendingBytes: Int
  public let emittedFrames: UInt64
  public let coalescedFrames: UInt64
  public let suppressedFrames: UInt64
  public let redactedNodes: UInt64
  public let evictedChunks: UInt64
  public let corruptChunks: UInt64
}
