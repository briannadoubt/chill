import CChillCompression
import CryptoKit
import Foundation

package struct StoredReplayChunk: Equatable, Sendable {
  package let descriptor: ChillReplayChunkDescriptor
  package let fileURL: URL
}

package struct ReplayStoreWriteResult: Sendable {
  package let stored: StoredReplayChunk
  package let evictedCount: Int
}

package final class ReplayChunkStore: @unchecked Sendable {
  private static let magic = Data("CHILLRP1\n".utf8)
  private static let headerByteCount = 13

  private let directory: URL
  private let corruptDirectory: URL
  private let key: SymmetricKey
  private let maximumDiskBytes: Int
  private let maximumChunkBytes: Int
  private let fileManager: FileManager
  private(set) var chunks: [StoredReplayChunk] = []
  private(set) var pendingBytes = 0
  private(set) var corruptChunks: UInt64 = 0

  package init(
    configuration: ChillReplayConfiguration,
    fileManager: FileManager = .default
  ) throws {
    directory = configuration.directory
    corruptDirectory = configuration.directory.appendingPathComponent(
      "corrupt",
      isDirectory: true
    )
    key = SymmetricKey(data: configuration.encryptionKey.bytes)
    maximumDiskBytes = configuration.maximumDiskBytes
    maximumChunkBytes = configuration.maximumChunkBytes
    self.fileManager = fileManager
    try fileManager.createDirectory(
      at: directory,
      withIntermediateDirectories: true
    )
    try fileManager.createDirectory(
      at: corruptDirectory,
      withIntermediateDirectories: true
    )
    try recover()
  }

  package func write(
    _ document: ReplayChunkDocument
  ) throws -> ReplayStoreWriteResult {
    let envelope = try encode(document)
    guard envelope.count <= maximumChunkBytes else {
      throw ChillReplayError.chunkTooLarge
    }
    let filename = String(
      format: "%020llu-%@.chillreplay",
      document.startMonotonicNano,
      document.chunkID
    )
    let destination = directory.appendingPathComponent(filename)
    var options: Data.WritingOptions = [.atomic]
    #if os(iOS) || os(tvOS) || os(watchOS) || os(visionOS)
      options.insert(.completeFileProtectionUntilFirstUserAuthentication)
    #endif
    try envelope.write(to: destination, options: options)
    let descriptor = Self.descriptor(
      for: document,
      encryptedEnvelope: envelope
    )
    let stored = StoredReplayChunk(
      descriptor: descriptor,
      fileURL: destination
    )
    chunks.append(stored)
    chunks.sort { $0.fileURL.lastPathComponent < $1.fileURL.lastPathComponent }
    pendingBytes += envelope.count
    var evicted = 0
    while pendingBytes > maximumDiskBytes, let oldest = chunks.first {
      try remove(oldest)
      evicted += 1
    }
    return ReplayStoreWriteResult(stored: stored, evictedCount: evicted)
  }

  package func descriptors() -> [ChillReplayChunkDescriptor] {
    chunks.map(\.descriptor)
  }

  package func encryptedData(chunkID: String) throws -> Data {
    guard
      let chunk = chunks.first(where: {
        $0.descriptor.chunkID == chunkID
      })
    else {
      throw ChillReplayError.chunkNotFound
    }
    return try Data(contentsOf: chunk.fileURL, options: [.mappedIfSafe])
  }

  package func decodedDocument(chunkID: String) throws -> ReplayChunkDocument {
    guard
      let chunk = chunks.first(where: {
        $0.descriptor.chunkID == chunkID
      })
    else {
      throw ChillReplayError.chunkNotFound
    }
    return try decode(Data(contentsOf: chunk.fileURL, options: [.mappedIfSafe]))
  }

  package func acknowledge(chunkIDs: Set<String>) throws -> Int {
    let acknowledged = chunks.filter {
      chunkIDs.contains($0.descriptor.chunkID)
    }
    for chunk in acknowledged { try remove(chunk) }
    return acknowledged.count
  }

  package func purge() throws {
    for chunk in chunks { try remove(chunk) }
    try removeReplayFiles(in: directory, extensions: ["chillreplay"])
    try removeReplayFiles(in: corruptDirectory, extensions: ["corrupt"])
    chunks.removeAll(keepingCapacity: false)
    pendingBytes = 0
  }

  private func recover() throws {
    let files = try fileManager.contentsOfDirectory(
      at: directory,
      includingPropertiesForKeys: nil,
      options: [.skipsHiddenFiles]
    )
    for file in files where file.pathExtension == "chillreplay" {
      do {
        let envelope = try Data(contentsOf: file, options: [.mappedIfSafe])
        guard envelope.count <= maximumChunkBytes else {
          throw ChillReplayError.chunkTooLarge
        }
        let document = try decode(envelope)
        if chunks.contains(where: {
          $0.descriptor.chunkID == document.chunkID
        }) {
          try fileManager.removeItem(at: file)
          continue
        }
        chunks.append(
          StoredReplayChunk(
            descriptor: Self.descriptor(
              for: document,
              encryptedEnvelope: envelope
            ),
            fileURL: file
          )
        )
        pendingBytes += envelope.count
      } catch {
        quarantine(file)
      }
    }
    chunks.sort { $0.fileURL.lastPathComponent < $1.fileURL.lastPathComponent }
    while pendingBytes > maximumDiskBytes, let oldest = chunks.first {
      try remove(oldest)
    }
  }

  private func encode(_ document: ReplayChunkDocument) throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    let cleartext = try encoder.encode(document)
    guard cleartext.count <= Int(UInt32.max),
      let compressed = ReplayGzip.compress(cleartext),
      let combined = try AES.GCM.seal(compressed, using: key).combined
    else {
      throw ChillReplayError.chunkTooLarge
    }
    var result = Data(capacity: Self.headerByteCount + combined.count)
    result.append(Self.magic)
    result.appendReplayUInt32(UInt32(cleartext.count))
    result.append(combined)
    return result
  }

  private func decode(_ envelope: Data) throws -> ReplayChunkDocument {
    guard envelope.count > Self.headerByteCount,
      envelope.prefix(Self.magic.count) == Self.magic
    else {
      throw ChillReplayError.corruptChunk
    }
    let cleartextCount = Int(envelope.readReplayUInt32(at: Self.magic.count))
    guard cleartextCount > 0,
      cleartextCount <= 16 * 1_024 * 1_024
    else {
      throw ChillReplayError.corruptChunk
    }
    let sealed = try AES.GCM.SealedBox(
      combined: envelope.dropFirst(Self.headerByteCount)
    )
    let compressed = try AES.GCM.open(sealed, using: key)
    guard
      let cleartext = ReplayGzip.decompress(
        compressed,
        expectedByteCount: cleartextCount
      )
    else {
      throw ChillReplayError.corruptChunk
    }
    do {
      return try JSONDecoder().decode(ReplayChunkDocument.self, from: cleartext)
    } catch {
      throw ChillReplayError.corruptChunk
    }
  }

  private func remove(_ chunk: StoredReplayChunk) throws {
    do {
      try fileManager.removeItem(at: chunk.fileURL)
    } catch CocoaError.fileNoSuchFile {
      // A prior upload acknowledgement already removed the immutable chunk.
    }
    if let index = chunks.firstIndex(where: {
      $0.fileURL == chunk.fileURL
    }) {
      let removed = chunks.remove(at: index)
      pendingBytes -= removed.descriptor.byteCount
    }
  }

  private func quarantine(_ file: URL) {
    corruptChunks &+= 1
    let destination = corruptDirectory.appendingPathComponent(
      "\(UUID().uuidString.lowercased()).corrupt"
    )
    do {
      try fileManager.moveItem(at: file, to: destination)
    } catch {
      try? fileManager.removeItem(at: file)
    }
  }

  private func removeReplayFiles(
    in directory: URL,
    extensions allowedExtensions: Set<String>
  ) throws {
    let files = try fileManager.contentsOfDirectory(
      at: directory,
      includingPropertiesForKeys: [.isRegularFileKey],
      options: [.skipsHiddenFiles]
    )
    for file in files where allowedExtensions.contains(file.pathExtension) {
      try fileManager.removeItem(at: file)
    }
  }

  private static func descriptor(
    for document: ReplayChunkDocument,
    encryptedEnvelope: Data
  ) -> ChillReplayChunkDescriptor {
    ChillReplayChunkDescriptor(
      replayID: document.replayID,
      chunkID: document.chunkID,
      chunkIndex: document.chunkIndex,
      sessionID: document.sessionID,
      bootID: document.bootID,
      startMonotonicNano: document.startMonotonicNano,
      endMonotonicNano: document.endMonotonicNano,
      occurredAtUnixNano: document.occurredAtUnixNano,
      digest: SHA256.hash(data: encryptedEnvelope).map {
        String(format: "%02x", $0)
      }.joined(),
      byteCount: encryptedEnvelope.count
    )
  }
}

private enum ReplayGzip {
  static func compress(_ data: Data) -> Data? {
    let capacity = chill_gzip_bound(data.count)
    guard capacity > 0 else { return nil }
    var destination = Data(count: capacity)
    var outputSize = capacity
    let status = data.withUnsafeBytes { sourceBuffer in
      destination.withUnsafeMutableBytes { destinationBuffer in
        chill_gzip_compress(
          sourceBuffer.bindMemory(to: UInt8.self).baseAddress,
          data.count,
          destinationBuffer.bindMemory(to: UInt8.self).baseAddress,
          &outputSize
        )
      }
    }
    guard status == 0, outputSize <= destination.count else { return nil }
    destination.removeSubrange(outputSize..<destination.count)
    return destination
  }

  static func decompress(
    _ data: Data,
    expectedByteCount: Int
  ) -> Data? {
    var destination = Data(count: expectedByteCount)
    var outputSize = expectedByteCount
    let status = data.withUnsafeBytes { sourceBuffer in
      destination.withUnsafeMutableBytes { destinationBuffer in
        chill_gzip_decompress(
          sourceBuffer.bindMemory(to: UInt8.self).baseAddress,
          data.count,
          destinationBuffer.bindMemory(to: UInt8.self).baseAddress,
          &outputSize
        )
      }
    }
    guard status == 0, outputSize == expectedByteCount else { return nil }
    return destination
  }
}

extension Data {
  fileprivate mutating func appendReplayUInt32(_ value: UInt32) {
    append(UInt8((value >> 24) & 0xFF))
    append(UInt8((value >> 16) & 0xFF))
    append(UInt8((value >> 8) & 0xFF))
    append(UInt8(value & 0xFF))
  }

  fileprivate func readReplayUInt32(at index: Int) -> UInt32 {
    (UInt32(self[index]) << 24)
      | (UInt32(self[index + 1]) << 16)
      | (UInt32(self[index + 2]) << 8)
      | UInt32(self[index + 3])
  }
}
