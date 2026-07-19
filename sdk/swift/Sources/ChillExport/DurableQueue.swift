import Foundation

package enum QueueDropReason: Equatable, Sendable {
  case capacity
  case corrupt
  case encoding
  case memoryBackpressure
}

package struct DroppedQueueRecord: Equatable, Sendable {
  package let recordID: String
  package let kind: String
  package let priority: ChillQueuePriority
  package let reason: QueueDropReason
}

package struct StoredQueueEntry: Equatable, Sendable {
  package let fileURL: URL
  package let recordID: String
  package let kind: String
  package let priority: ChillQueuePriority
  package let payload: Data
  package let storedBytes: Int
}

package struct QueueAppendResult: Sendable {
  package let persisted: Bool
  package let duplicate: Bool
  package let dropped: [DroppedQueueRecord]
}

// Production mutation is confined to OfflineExportWorker's actor. Tests create
// an isolated queue instance and never share it across executors.
package final class DurableRecordQueue: @unchecked Sendable {
  private static let magic = Data("CHILLQ1\n".utf8)
  private static let headerByteCount = 20

  private let directory: URL
  private let corruptDirectory: URL
  private let maximumBytes: Int
  private let fileManager: FileManager
  private(set) var entries: [StoredQueueEntry] = []
  private(set) var queuedBytes = 0
  private(set) var recoveredRecords = 0
  private(set) var quarantinedFiles = 0

  package init(
    directory: URL,
    maximumBytes: Int,
    fileManager: FileManager = .default
  ) throws {
    self.directory = directory
    self.maximumBytes = maximumBytes
    self.fileManager = fileManager
    corruptDirectory = directory.appendingPathComponent(
      "corrupt",
      isDirectory: true
    )
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

  package var count: Int { entries.count }

  package func append(
    recordID: String,
    kind: String,
    priority: ChillQueuePriority,
    occurredAtUnixNano: UInt64,
    sequenceNumber: UInt64,
    payload: Data
  ) throws -> QueueAppendResult {
    if entries.contains(where: { $0.recordID == recordID }) {
      return QueueAppendResult(
        persisted: false,
        duplicate: true,
        dropped: []
      )
    }
    let encoded = try Self.encode(
      recordID: recordID,
      kind: kind,
      priority: priority,
      payload: payload
    )
    guard encoded.count <= maximumBytes else {
      return QueueAppendResult(
        persisted: false,
        duplicate: false,
        dropped: [
          DroppedQueueRecord(
            recordID: recordID,
            kind: kind,
            priority: priority,
            reason: .capacity
          )
        ]
      )
    }

    var victims: [StoredQueueEntry] = []
    var projectedBytes = queuedBytes + encoded.count
    while projectedBytes > maximumBytes {
      guard let victim = evictionCandidate(for: priority, excluding: victims)
      else {
        return QueueAppendResult(
          persisted: false,
          duplicate: false,
          dropped: [
            DroppedQueueRecord(
              recordID: recordID,
              kind: kind,
              priority: priority,
              reason: .capacity
            )
          ]
        )
      }
      victims.append(victim)
      projectedBytes -= victim.storedBytes
    }

    let filename = String(
      format: "%020llu-%020llu-%@.chillq",
      occurredAtUnixNano,
      sequenceNumber,
      UUID().uuidString.lowercased()
    )
    let destination = directory.appendingPathComponent(filename)
    try writeAtomically(encoded, to: destination)
    let stored = StoredQueueEntry(
      fileURL: destination,
      recordID: recordID,
      kind: kind,
      priority: priority,
      payload: payload,
      storedBytes: encoded.count
    )
    entries.append(stored)
    entries.sort { $0.fileURL.lastPathComponent < $1.fileURL.lastPathComponent }
    queuedBytes += encoded.count

    var dropped: [DroppedQueueRecord] = []
    for victim in victims {
      try remove(victim)
      dropped.append(
        DroppedQueueRecord(
          recordID: victim.recordID,
          kind: victim.kind,
          priority: victim.priority,
          reason: .capacity
        )
      )
    }
    return QueueAppendResult(
      persisted: true,
      duplicate: false,
      dropped: dropped
    )
  }

  package func peek(
    maximumRecords: Int,
    maximumPayloadBytes: Int
  ) -> [StoredQueueEntry] {
    var selected: [StoredQueueEntry] = []
    var payloadBytes = 0
    for entry in entries where selected.count < maximumRecords {
      let nextBytes = payloadBytes + entry.payload.count
      if !selected.isEmpty, nextBytes > maximumPayloadBytes { break }
      selected.append(entry)
      payloadBytes = nextBytes
    }
    return selected
  }

  package func acknowledge(recordIDs: Set<String>) throws -> Int {
    guard !recordIDs.isEmpty else { return 0 }
    let acknowledged = entries.filter { recordIDs.contains($0.recordID) }
    for entry in acknowledged { try remove(entry) }
    return acknowledged.count
  }

  private func recover() throws {
    let urls = try fileManager.contentsOfDirectory(
      at: directory,
      includingPropertiesForKeys: nil,
      options: [.skipsHiddenFiles]
    )
    for url in urls where url.pathExtension == "chillq" {
      do {
        let data = try Data(contentsOf: url, options: [.mappedIfSafe])
        let decoded = try Self.decode(data, fileURL: url)
        if entries.contains(where: { $0.recordID == decoded.recordID }) {
          try fileManager.removeItem(at: url)
          continue
        }
        entries.append(decoded)
        queuedBytes += decoded.storedBytes
      } catch {
        quarantine(url)
      }
    }
    entries.sort { $0.fileURL.lastPathComponent < $1.fileURL.lastPathComponent }
    recoveredRecords = entries.count
    try enforceRecoveredCapacity()
  }

  private func enforceRecoveredCapacity() throws {
    while queuedBytes > maximumBytes, let victim = recoveryEvictionCandidate() {
      try remove(victim)
    }
  }

  private func evictionCandidate(
    for incoming: ChillQueuePriority,
    excluding: [StoredQueueEntry]
  ) -> StoredQueueEntry? {
    let excluded = Set(excluding.map(\.fileURL))
    return
      entries
      .filter { entry in
        guard !excluded.contains(entry.fileURL) else { return false }
        if entry.priority.rawValue < incoming.rawValue { return true }
        return entry.priority == incoming && incoming != .high
      }
      .min { left, right in
        if left.priority != right.priority {
          return left.priority.rawValue < right.priority.rawValue
        }
        return left.fileURL.lastPathComponent < right.fileURL.lastPathComponent
      }
  }

  private func recoveryEvictionCandidate() -> StoredQueueEntry? {
    entries.min { left, right in
      if left.priority != right.priority {
        return left.priority.rawValue < right.priority.rawValue
      }
      return left.fileURL.lastPathComponent < right.fileURL.lastPathComponent
    }
  }

  private func remove(_ entry: StoredQueueEntry) throws {
    do {
      try fileManager.removeItem(at: entry.fileURL)
    } catch CocoaError.fileNoSuchFile {
      // A prior acknowledgement or recovery cleanup already removed it.
    }
    if let index = entries.firstIndex(where: { $0.fileURL == entry.fileURL }) {
      let removed = entries.remove(at: index)
      queuedBytes -= removed.storedBytes
    }
  }

  private func quarantine(_ url: URL) {
    quarantinedFiles += 1
    let destination = corruptDirectory.appendingPathComponent(
      "\(UUID().uuidString.lowercased()).corrupt"
    )
    do {
      try fileManager.moveItem(at: url, to: destination)
    } catch {
      try? fileManager.removeItem(at: url)
    }
  }

  private func writeAtomically(_ data: Data, to url: URL) throws {
    var options: Data.WritingOptions = [.atomic]
    #if os(iOS) || os(tvOS) || os(watchOS) || os(visionOS)
      options.insert(.completeFileProtectionUntilFirstUserAuthentication)
    #endif
    try data.write(to: url, options: options)
  }

  private static func encode(
    recordID: String,
    kind: String,
    priority: ChillQueuePriority,
    payload: Data
  ) throws -> Data {
    let recordIDBytes = Data(recordID.utf8)
    let kindBytes = Data(kind.utf8)
    guard recordIDBytes.count <= Int(UInt16.max),
      kindBytes.count <= Int(UInt8.max),
      payload.count <= Int(UInt32.max)
    else {
      throw ChillExportConfigurationError.invalidQueueLimit
    }
    var result = Data(
      capacity: headerByteCount + recordIDBytes.count + kindBytes.count + payload.count)
    result.append(magic)
    result.append(priority.rawValue)
    result.appendUInt16(UInt16(recordIDBytes.count))
    result.append(UInt8(kindBytes.count))
    result.appendUInt32(UInt32(payload.count))
    result.appendUInt32(CRC32.checksum(recordIDBytes + kindBytes + payload))
    result.append(recordIDBytes)
    result.append(kindBytes)
    result.append(payload)
    return result
  }

  private static func decode(_ data: Data, fileURL: URL) throws -> StoredQueueEntry {
    guard data.count >= headerByteCount,
      data.prefix(magic.count) == magic
    else { throw QueueFileError.corrupt }
    let priorityOffset = magic.count
    guard let priority = ChillQueuePriority(rawValue: data[priorityOffset])
    else { throw QueueFileError.corrupt }
    let idLength = Int(data.readUInt16(at: priorityOffset + 1))
    let kindLength = Int(data[priorityOffset + 3])
    let payloadLength = Int(data.readUInt32(at: priorityOffset + 4))
    let checksum = data.readUInt32(at: priorityOffset + 8)
    let expectedLength = headerByteCount + idLength + kindLength + payloadLength
    guard expectedLength == data.count else { throw QueueFileError.corrupt }
    let idStart = headerByteCount
    let kindStart = idStart + idLength
    let payloadStart = kindStart + kindLength
    let idData = data[idStart..<kindStart]
    let kindData = data[kindStart..<payloadStart]
    let payload = Data(data[payloadStart..<data.endIndex])
    guard CRC32.checksum(Data(idData) + Data(kindData) + payload) == checksum,
      let recordID = String(data: idData, encoding: .utf8),
      !recordID.isEmpty,
      let kind = String(data: kindData, encoding: .utf8),
      !kind.isEmpty
    else { throw QueueFileError.corrupt }
    return StoredQueueEntry(
      fileURL: fileURL,
      recordID: recordID,
      kind: kind,
      priority: priority,
      payload: payload,
      storedBytes: data.count
    )
  }
}

private enum QueueFileError: Error {
  case corrupt
}

private enum CRC32 {
  static let table: [UInt32] = (0..<256).map { value in
    var crc = UInt32(value)
    for _ in 0..<8 {
      crc = crc & 1 == 1 ? 0xEDB8_8320 ^ (crc >> 1) : crc >> 1
    }
    return crc
  }

  static func checksum(_ data: Data) -> UInt32 {
    var crc = UInt32.max
    for byte in data {
      let index = Int((crc ^ UInt32(byte)) & 0xFF)
      crc = table[index] ^ (crc >> 8)
    }
    return crc ^ UInt32.max
  }
}

extension Data {
  fileprivate mutating func appendUInt16(_ value: UInt16) {
    append(UInt8((value >> 8) & 0xFF))
    append(UInt8(value & 0xFF))
  }

  fileprivate mutating func appendUInt32(_ value: UInt32) {
    append(UInt8((value >> 24) & 0xFF))
    append(UInt8((value >> 16) & 0xFF))
    append(UInt8((value >> 8) & 0xFF))
    append(UInt8(value & 0xFF))
  }

  fileprivate func readUInt16(at index: Int) -> UInt16 {
    (UInt16(self[index]) << 8) | UInt16(self[index + 1])
  }

  fileprivate func readUInt32(at index: Int) -> UInt32 {
    (UInt32(self[index]) << 24)
      | (UInt32(self[index + 1]) << 16)
      | (UInt32(self[index + 2]) << 8)
      | UInt32(self[index + 3])
  }
}
