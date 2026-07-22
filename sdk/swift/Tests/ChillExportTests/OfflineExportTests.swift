import Foundation
import Testing
import os

@_spi(TraceIntegration) @testable import ChillCore
@testable import ChillExport

private func temporaryDirectory(_ name: String = #function) throws -> URL {
  let url = FileManager.default.temporaryDirectory.appendingPathComponent(
    "chill-export-tests-\(name)-\(UUID().uuidString)",
    isDirectory: true
  )
  try FileManager.default.createDirectory(
    at: url,
    withIntermediateDirectories: true
  )
  return url
}

private func makeActionRecord(
  id: String,
  sequence: UInt64,
  annotationValue: String = "internal",
  trace: TraceContext? = nil,
  monotonicNano: UInt64? = nil,
  durationNano: UInt64? = nil
) throws -> BehaviorRecord {
  let declaredAnnotations = try AnnotationContext().addingScope(
    id: AnnotationScopeID("test"),
    declarations: [
      try AnnotationDeclaration("release.channel", value: annotationValue)
    ]
  ).snapshot
  let annotations = try PrivacyPolicy(
    version: "test-v1",
    analytics: .granted,
    annotationAllowlist: ["release.channel": .internalData]
  ).classifyAnnotations(declaredAnnotations).snapshot
  return BehaviorRecord(
    schemaVersion: "1.0.0",
    schemaURL: "https://schemas.chill.dev/behavior/v1/envelope.schema.json",
    recordID: try RecordID(id),
    subjectID: try SubjectID("action-\(sequence)"),
    sessionID: try SessionID("session-test"),
    kind: .action,
    operation: .instant,
    name: try SemanticName("cat.adopt"),
    clock: RecordClock(
      occurredAtUnixNano: 1_000 + sequence,
      observedAtUnixNano: 1_100 + sequence,
      monotonicNano: monotonicNano ?? 500 + sequence,
      bootID: try BootID("boot-test"),
      sequenceNumber: sequence
    ),
    annotations: annotations,
    page: nil,
    element: nil,
    trace: trace,
    captureClass: .analytics,
    consent: .granted,
    policyVersion: "test-v1",
    redactionState: .none,
    redactionCount: 0,
    payload: .action(
      ActionPayload(
        elementID: try SemanticName("cat.adopt"),
        role: try SemanticName("button"),
        activation: .primary,
        input: .touch
      )
    ),
    durationNano: durationNano
  )
}

private func makeImpressionRecord(visibleDurationNano: UInt64) throws -> BehaviorRecord {
  BehaviorRecord(
    schemaVersion: "1.0.0",
    schemaURL: "https://schemas.chill.dev/behavior/v1/envelope.schema.json",
    recordID: try RecordID("record-impression-wire"),
    subjectID: try SubjectID("impression-wire"),
    sessionID: try SessionID("session-test"),
    kind: .impression,
    operation: .instant,
    name: try SemanticName("cat.card.visible"),
    clock: RecordClock(
      occurredAtUnixNano: 1_784_687_848_652_707_200,
      observedAtUnixNano: 1_784_687_848_652_707_300,
      monotonicNano: 1_784_687_848_652_707_400,
      bootID: try BootID("boot-test"),
      sequenceNumber: 2
    ),
    annotations: .empty,
    page: nil,
    element: nil,
    trace: nil,
    captureClass: .analytics,
    consent: .granted,
    policyVersion: "test-v1",
    redactionState: .none,
    redactionCount: 0,
    payload: .impression(
      try ImpressionPayload(
        elementID: try SemanticName("cat.card"),
        role: try SemanticName("card"),
        visibilityRatio: 1,
        visibleDurationNano: visibleDurationNano
      )
    ),
    durationNano: nil
  )
}

private func makeReplayRecord() throws -> BehaviorRecord {
  BehaviorRecord(
    schemaVersion: "1.0.0",
    schemaURL: "https://schemas.chill.dev/behavior/v1/envelope.schema.json",
    recordID: try RecordID("record-replay-wire"),
    subjectID: try SubjectID("replay-wire"),
    sessionID: try SessionID("session-test"),
    kind: .replay,
    operation: .instant,
    name: try SemanticName("session.replay"),
    clock: RecordClock(
      occurredAtUnixNano: 1_784_687_848_652_707_200,
      observedAtUnixNano: 1_784_687_848_652_707_300,
      monotonicNano: 500,
      bootID: try BootID("boot-test"),
      sequenceNumber: 1
    ),
    annotations: .empty,
    page: nil,
    element: nil,
    trace: nil,
    captureClass: .replay,
    consent: .granted,
    policyVersion: "test-v1",
    redactionState: .applied,
    redactionCount: 0,
    payload: .replay(
      try ReplayPayload(
        replayID: "replay-wire",
        chunkID: "chunk-wire",
        chunkIndex: 0,
        startsAtUnixNano: 1_784_687_848_652_707_200,
        endsAtUnixNano: 1_784_687_848_652_707_300,
        sha256: String(repeating: "a", count: 64),
        byteCount: 128,
        storageRef: "replay://chunk/chunk-wire"
      )
    ),
    durationNano: nil
  )
}

private final class ScriptedTransport: OTLPExportTransport,
  @unchecked Sendable
{
  private struct State {
    var dispositions: [OTLPExportDisposition]
    var requests: [OTLPExportRequest] = []
  }

  private let state: OSAllocatedUnfairLock<State>

  init(_ dispositions: [OTLPExportDisposition]) {
    state = OSAllocatedUnfairLock(
      initialState: State(dispositions: dispositions)
    )
  }

  func export(_ request: OTLPExportRequest) async -> OTLPExportDisposition {
    state.withLock { state in
      state.requests.append(request)
      if state.dispositions.isEmpty {
        return .acknowledged(request.recordIDs)
      }
      return state.dispositions.removeFirst()
    }
  }

  var requests: [OTLPExportRequest] { state.withLock { $0.requests } }
}

private struct ExportStubResponse {
  let statusCode: Int
  let headers: [String: String]
}

private final class ExportURLProtocol: URLProtocol, @unchecked Sendable {
  nonisolated(unsafe) static var handler: ((URLRequest) -> ExportStubResponse)?

  override class func canInit(with _: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest {
    request
  }

  override func startLoading() {
    guard let result = Self.handler?(request) else {
      client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
      return
    }
    let response = HTTPURLResponse(
      url: request.url!,
      statusCode: result.statusCode,
      httpVersion: "HTTP/1.1",
      headerFields: result.headers
    )!
    client?.urlProtocol(
      self,
      didReceive: response,
      cacheStoragePolicy: .notAllowed
    )
    client?.urlProtocolDidFinishLoading(self)
  }

  override func stopLoading() {}
}

private struct ProtobufField {
  let number: Int
  let wireType: UInt64
  let value: Data
}

private enum ProtobufTestError: Error {
  case invalid
}

private func protobufFields(_ data: Data) throws -> [ProtobufField] {
  let bytes = Array(data)
  var index = 0
  var fields: [ProtobufField] = []
  while index < bytes.count {
    let tag = try readVarint(bytes, index: &index)
    let number = Int(tag >> 3)
    let wire = tag & 7
    let value: Data
    switch wire {
    case 0:
      let start = index
      _ = try readVarint(bytes, index: &index)
      value = Data(bytes[start..<index])
    case 1:
      guard index + 8 <= bytes.count else { throw ProtobufTestError.invalid }
      value = Data(bytes[index..<index + 8])
      index += 8
    case 2:
      let length = Int(try readVarint(bytes, index: &index))
      guard length >= 0, index + length <= bytes.count else {
        throw ProtobufTestError.invalid
      }
      value = Data(bytes[index..<index + length])
      index += length
    case 5:
      guard index + 4 <= bytes.count else { throw ProtobufTestError.invalid }
      value = Data(bytes[index..<index + 4])
      index += 4
    default:
      throw ProtobufTestError.invalid
    }
    fields.append(ProtobufField(number: number, wireType: wire, value: value))
  }
  return fields
}

private func readVarint(_ bytes: [UInt8], index: inout Int) throws -> UInt64 {
  var value: UInt64 = 0
  for shift in stride(from: 0, through: 63, by: 7) {
    guard index < bytes.count else { throw ProtobufTestError.invalid }
    let byte = bytes[index]
    index += 1
    value |= UInt64(byte & 0x7F) << UInt64(shift)
    if byte & 0x80 == 0 { return value }
  }
  throw ProtobufTestError.invalid
}

private func requestBody(_ request: URLRequest) -> Data? {
  if let body = request.httpBody { return body }
  guard let stream = request.httpBodyStream else { return nil }
  stream.open()
  defer { stream.close() }
  var result = Data()
  var buffer = [UInt8](repeating: 0, count: 4_096)
  while true {
    let count = stream.read(&buffer, maxLength: buffer.count)
    if count < 0 { return nil }
    if count == 0 { break }
    result.append(contentsOf: buffer.prefix(count))
  }
  return result
}

@Suite("Crash-safe offline queue")
struct DurableQueueTests {
  @Test("Recovery preserves IDs and acknowledgements are the only removal")
  func atLeastOnceRecoveryAndPriorityEviction() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }

    var queue: DurableRecordQueue? = try DurableRecordQueue(
      directory: directory,
      maximumBytes: 600
    )
    _ = try queue?.append(
      recordID: "record-action",
      kind: "action",
      priority: .high,
      occurredAtUnixNano: 1,
      sequenceNumber: 1,
      payload: Data(repeating: 1, count: 120)
    )
    _ = try queue?.append(
      recordID: "record-impression",
      kind: "impression",
      priority: .low,
      occurredAtUnixNano: 2,
      sequenceNumber: 2,
      payload: Data(repeating: 2, count: 100)
    )
    _ = try queue?.append(
      recordID: "record-replay",
      kind: "replay",
      priority: .replay,
      occurredAtUnixNano: 3,
      sequenceNumber: 3,
      payload: Data(repeating: 3, count: 200)
    )
    let capacityMutation = try queue?.append(
      recordID: "record-event",
      kind: "event",
      priority: .high,
      occurredAtUnixNano: 4,
      sequenceNumber: 4,
      payload: Data(repeating: 4, count: 120)
    )
    #expect(capacityMutation?.dropped.map(\.recordID) == ["record-replay"])
    #expect(queue?.queuedBytes ?? .max <= 600)

    queue = nil  // Simulate process death without an acknowledgement.
    let recovered = try DurableRecordQueue(
      directory: directory,
      maximumBytes: 600
    )
    #expect(recovered.recoveredRecords == 3)
    #expect(
      recovered.peek(maximumRecords: 2, maximumPayloadBytes: 10_000)
        .map(\.recordID) == ["record-action", "record-impression"]
    )
    // A failed export changes nothing, so the next attempt is byte-for-byte
    // the same ordered batch.
    #expect(
      recovered.peek(maximumRecords: 2, maximumPayloadBytes: 10_000)
        .map(\.recordID) == ["record-action", "record-impression"]
    )
    #expect(
      try recovered.acknowledge(recordIDs: ["record-action"]) == 1
    )
    _ = try recovered.append(
      recordID: "record-activity-end",
      kind: "activity",
      priority: .high,
      occurredAtUnixNano: 5,
      sequenceNumber: 5,
      payload: Data(repeating: 5, count: 140)
    )
    #expect(
      recovered.entries.map(\.recordID) == [
        "record-impression", "record-event", "record-activity-end",
      ]
    )
  }

  @Test("Duplicate IDs do not create a second durable fact")
  func duplicateIDIsIdempotent() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let queue = try DurableRecordQueue(directory: directory, maximumBytes: 500)
    _ = try queue.append(
      recordID: "record-one",
      kind: "event",
      priority: .high,
      occurredAtUnixNano: 1,
      sequenceNumber: 1,
      payload: Data("one".utf8)
    )
    let duplicate = try queue.append(
      recordID: "record-one",
      kind: "event",
      priority: .high,
      occurredAtUnixNano: 2,
      sequenceNumber: 2,
      payload: Data("different".utf8)
    )
    #expect(duplicate.duplicate)
    #expect(!duplicate.persisted)
    #expect(queue.count == 1)
  }

  @Test("Torn or corrupted files are quarantined during recovery")
  func corruptedFilesAreQuarantined() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    try Data("CHILLQ1\nbroken".utf8).write(
      to: directory.appendingPathComponent("broken.chillq")
    )

    let queue = try DurableRecordQueue(directory: directory, maximumBytes: 500)
    #expect(queue.count == 0)
    #expect(queue.quarantinedFiles == 1)
    let corrupt = directory.appendingPathComponent("corrupt")
    #expect(try FileManager.default.contentsOfDirectory(atPath: corrupt.path).count == 1)
  }
}

@Suite("OTLP offline exporter", .serialized)
struct OfflineExporterTests {
  @Test("Exporter configuration fails closed at trust and resource bounds")
  func configurationBounds() {
    #expect(throws: ChillExportConfigurationError.invalidEndpoint) {
      _ = try ChillOTLPConfiguration(
        endpoint: URL(string: "http://collector.example.test/v1/logs")!
      )
    }
    #expect(throws: ChillExportConfigurationError.invalidEndpoint) {
      _ = try ChillOTLPConfiguration(
        endpoint: URL(string: "https://user:secret@collector.example.test/v1/logs")!
      )
    }
    #expect(throws: ChillExportConfigurationError.invalidHeader) {
      _ = try ChillOTLPConfiguration(
        endpoint: URL(string: "https://collector.example.test/v1/logs")!,
        headers: ["Authorization": "Bearer safe\r\ninjected: value"]
      )
    }
    #expect(throws: ChillExportConfigurationError.invalidQueueLimit) {
      _ = try ChillOfflineConfiguration(
        maximumDiskBytes: 64 * 1_024 * 1_024 + 1
      )
    }
  }

  @Test("An empty queue never schedules an idle request")
  func noIdleRequests() async throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let offline = try ChillOfflineConfiguration(
      maximumDiskBytes: 64 * 1_024,
      maximumMemoryRecords: 16,
      maximumBatchRecords: 10,
      maximumBatchBytes: 16 * 1_024,
      flushInterval: 0.01
    )
    let configuration = try ChillOTLPConfiguration(
      endpoint: URL(string: "http://localhost:4318/v1/logs")!,
      offline: offline,
      allowInsecureLocalhost: true
    )
    let queue = try DurableRecordQueue(
      directory: directory,
      maximumBytes: offline.maximumDiskBytes
    )
    let transport = ScriptedTransport([])
    let pipeline = ChillOfflinePipeline(
      configuration: configuration,
      queue: queue,
      transport: transport
    )

    try await Task.sleep(for: .milliseconds(30))
    #expect(await pipeline.metrics().exportAttempts == 0)
    #expect(transport.requests.isEmpty)
    #expect(await pipeline.shutdown() == .emptied)
  }

  @Test("Retry keeps a batch and a later acknowledgement removes exact IDs")
  func retryAndAcknowledgement() async throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let offline = try ChillOfflineConfiguration(
      maximumDiskBytes: 64 * 1_024,
      maximumMemoryRecords: 16,
      maximumBatchRecords: 100,
      maximumBatchBytes: 16 * 1_024,
      flushInterval: 300,
      retry: ChillRetryConfiguration(initialDelay: 60)
    )
    let configuration = try ChillOTLPConfiguration(
      endpoint: URL(string: "http://localhost:4318/v1/logs")!,
      resourceAttributes: ["service.name": "chill-tests"],
      offline: offline,
      allowInsecureLocalhost: true
    )
    let queue = try DurableRecordQueue(
      directory: directory,
      maximumBytes: offline.maximumDiskBytes
    )
    let transport = ScriptedTransport([
      .retry(after: 60)
    ])
    let pipeline = ChillOfflinePipeline(
      configuration: configuration,
      queue: queue,
      transport: transport
    )
    pipeline.submit(try makeActionRecord(id: "record-1", sequence: 1))
    pipeline.submit(try makeActionRecord(id: "record-2", sequence: 2))

    #expect(await pipeline.flush() == .retryScheduled)
    var metrics = await pipeline.metrics()
    #expect(metrics.queuedRecords == 2)
    #expect(metrics.exportAttempts == 1)
    #expect(metrics.exportFailures == 1)
    #expect(await pipeline.flush() == .emptied)
    metrics = await pipeline.metrics()
    #expect(metrics.queuedRecords == 0)
    #expect(metrics.acknowledgedRecords == 2)
    #expect(transport.requests.count == 2)
    #expect(transport.requests[0].recordIDs == transport.requests[1].recordIDs)
  }

  @Test("A thousand-record burst retains every high-value fact")
  func highValueBurstHasNoDrops() async throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let offline = try ChillOfflineConfiguration(
      maximumDiskBytes: 64 * 1_024 * 1_024,
      maximumMemoryRecords: 2_048,
      maximumBatchRecords: 1_000,
      maximumBatchBytes: 4 * 1_024 * 1_024,
      flushInterval: 300
    )
    let configuration = try ChillOTLPConfiguration(
      endpoint: URL(string: "http://localhost:4318/v1/logs")!,
      offline: offline,
      allowInsecureLocalhost: true
    )
    let queue = try DurableRecordQueue(
      directory: directory,
      maximumBytes: offline.maximumDiskBytes
    )
    let transport = ScriptedTransport([])
    let pipeline = ChillOfflinePipeline(
      configuration: configuration,
      queue: queue,
      transport: transport
    )
    for sequence in 1...1_000 {
      pipeline.submit(
        try makeActionRecord(
          id: "record-burst-\(sequence)",
          sequence: UInt64(sequence)
        )
      )
    }

    #expect(await pipeline.flush() == .emptied)
    let metrics = await pipeline.metrics()
    #expect(metrics.persistedRecords == 1_000)
    #expect(metrics.acknowledgedRecords == 1_000)
    #expect(metrics.droppedHighPriorityRecords == 0)
    #expect(metrics.queuedRecords == 0)
    #expect(await pipeline.shutdown() == .emptied)
  }

  @Test("Log encoding is lossless, typed, and stable")
  func losslessOTLPLogEncoding() throws {
    let record = try makeActionRecord(id: "record-wire", sequence: 7)
    let data = try OTLPJSONEncoder.encodeLogRecord(record)
    let object = try #require(
      JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    #expect(object["eventName"] as? String == "app.widget.click")
    #expect(object["timeUnixNano"] as? String == "1007")
    let attributes = try #require(object["attributes"] as? [[String: Any]])
    let decoded = decodeAttributes(attributes)
    #expect(decoded["chill.record.id"] as? String == "record-wire")
    #expect(decoded["chill.annotation.release.channel"] as? String == "internal")
    #expect(decoded["chill.clock.sequence_number"] as? String == "7")
    #expect(decoded["chill.payload.input"] as? String == "touch")
  }

  @Test("Production protobuf uses the stable OTLP Logs field numbers")
  func protobufWireShape() throws {
    let trace = try TraceContext(
      traceID: "4bf92f3577b34da6a3ce929d0e0e4736",
      spanID: "00f067aa0ba902b7",
      traceFlags: 1
    )
    let record = try makeActionRecord(
      id: "record-protobuf",
      sequence: 8,
      trace: trace
    )
    let encoded = OTLPProtobufEncoder.encodeLogRecord(record)
    // One exact data-classification attribute is security metadata, not user
    // payload. Keep the raw single-record envelope bounded independently from
    // the stricter compressed batch budget.
    #expect(encoded.count <= 1_100)
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let queue = try DurableRecordQueue(directory: directory, maximumBytes: 10_000)
    _ = try queue.append(
      recordID: record.recordID.rawValue,
      kind: record.kind.rawValue,
      priority: .high,
      occurredAtUnixNano: record.clock.occurredAtUnixNano,
      sequenceNumber: record.clock.sequenceNumber,
      payload: encoded
    )
    #expect(queue.queuedBytes <= 1_100)
    let fields = try protobufFields(encoded)
    #expect(fields.first { $0.number == 1 }?.wireType == 1)
    #expect(fields.filter { $0.number == 6 }.count >= 20)
    #expect(fields.first { $0.number == 8 }?.wireType == 5)
    #expect(fields.first { $0.number == 9 }?.value.count == 16)
    #expect(fields.first { $0.number == 10 }?.value.count == 8)
    #expect(fields.first { $0.number == 11 }?.wireType == 1)
    #expect(
      fields.first { $0.number == 12 }.flatMap {
        String(data: $0.value, encoding: .utf8)
      } == "app.widget.click"
    )
    let attributeKeys = try fields.filter { $0.number == 6 }.compactMap {
      try protobufFields($0.value).first { $0.number == 1 }.flatMap {
        String(data: $0.value, encoding: .utf8)
      }
    }
    #expect(attributeKeys.contains("chill.record.id"))
    #expect(attributeKeys.contains("chill.annotation.release.channel"))
  }

  @Test("Replay nanosecond timestamps preserve the canonical uint64 string contract")
  func replayTimestampsAreDecimalStrings() throws {
    let data = try OTLPJSONEncoder.encodeLogRecord(makeReplayRecord())
    let object = try #require(
      JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let attributes = try #require(object["attributes"] as? [[String: Any]])
    let decoded = decodeAttributes(attributes)

    #expect(
      decoded["chill.payload.starts_at_unix_nano"] as? String
        == "1784687848652707200"
    )
    #expect(
      decoded["chill.payload.ends_at_unix_nano"] as? String
        == "1784687848652707300"
    )
  }

  @Test("Every projected nanosecond value preserves the canonical uint64 string contract")
  func projectedNanosecondsAreDecimalStrings() throws {
    let monotonic = UInt64.max - 2
    let duration = UInt64.max - 1
    let visibleDuration = UInt64.max
    let action = try makeActionRecord(
      id: "record-nanosecond-wire",
      sequence: 9,
      monotonicNano: monotonic,
      durationNano: duration
    )
    let actionObject = try #require(
      JSONSerialization.jsonObject(
        with: OTLPJSONEncoder.encodeLogRecord(action)
      ) as? [String: Any]
    )
    let actionAttributes = decodeAttributes(
      try #require(actionObject["attributes"] as? [[String: Any]])
    )
    #expect(
      actionAttributes["chill.clock.monotonic_nano"] as? String
        == String(monotonic)
    )
    #expect(
      actionAttributes["chill.duration_nano"] as? String
        == String(duration)
    )

    let impressionObject = try #require(
      JSONSerialization.jsonObject(
        with: OTLPJSONEncoder.encodeLogRecord(
          makeImpressionRecord(visibleDurationNano: visibleDuration)
        )
      ) as? [String: Any]
    )
    let impressionAttributes = decodeAttributes(
      try #require(impressionObject["attributes"] as? [[String: Any]])
    )
    #expect(
      impressionAttributes["chill.payload.visible_duration_nano"] as? String
        == String(visibleDuration)
    )
  }

  @Test("Batch encoding shares one resource and instrumentation scope")
  func batchedResourceAndScope() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let queue = try DurableRecordQueue(directory: directory, maximumBytes: 50_000)
    for sequence in 1...2 {
      let record = try makeActionRecord(
        id: "record-batch-\(sequence)",
        sequence: UInt64(sequence)
      )
      _ = try queue.append(
        recordID: record.recordID.rawValue,
        kind: record.kind.rawValue,
        priority: .high,
        occurredAtUnixNano: record.clock.occurredAtUnixNano,
        sequenceNumber: record.clock.sequenceNumber,
        payload: OTLPJSONEncoder.encodeLogRecord(record)
      )
    }
    let batch = try OTLPJSONEncoder.encodeBatch(
      queue.entries,
      resourceAttributes: ["service.name": "cats"]
    )
    let root = try #require(
      JSONSerialization.jsonObject(with: batch) as? [String: Any]
    )
    let resourceLogs = try #require(root["resourceLogs"] as? [[String: Any]])
    #expect(resourceLogs.count == 1)
    let scopeLogs = try #require(resourceLogs[0]["scopeLogs"] as? [[String: Any]])
    #expect(scopeLogs.count == 1)
    let records = try #require(scopeLogs[0]["logRecords"] as? [[String: Any]])
    #expect(records.count == 2)

    let protobufEntries = try queue.entries.enumerated().map { index, entry in
      let record = try makeActionRecord(
        id: entry.recordID,
        sequence: UInt64(index + 1)
      )
      return StoredQueueEntry(
        fileURL: entry.fileURL,
        recordID: entry.recordID,
        kind: entry.kind,
        priority: entry.priority,
        payload: OTLPProtobufEncoder.encodeLogRecord(record),
        storedBytes: entry.storedBytes
      )
    }
    let protobuf = OTLPProtobufEncoder.encodeBatch(
      protobufEntries,
      resourceAttributes: ["service.name": "cats"]
    )
    let requestFields = try protobufFields(protobuf)
    let protobufResourceLogs = try #require(
      requestFields.first { $0.number == 1 }
    )
    let resourceLogFields = try protobufFields(protobufResourceLogs.value)
    let protobufScopeLogs = try #require(
      resourceLogFields.first { $0.number == 2 }
    )
    let scopeFields = try protobufFields(protobufScopeLogs.value)
    #expect(scopeFields.filter { $0.number == 2 }.count == 2)
  }

  @Test("Compression round-trips and retry jitter remains bounded")
  func compressionAndRetryBounds() throws {
    let source = Data(String(repeating: "chill-behavior-record\n", count: 500).utf8)
    let compressed = try #require(GzipCompression.compress(source))
    #expect(compressed.count < source.count)
    #expect(compressed.prefix(2) == Data([0x1F, 0x8B]))
    #expect(
      GzipCompression.decompress(
        compressed,
        expectedByteCount: source.count
      ) == source
    )
    let retry = try ChillRetryConfiguration(
      initialDelay: 1,
      maximumDelay: 10,
      multiplier: 2,
      jitterRatio: 0.2
    )
    #expect(retry.delay(attempt: 2, unitRandom: 0) == 3.2)
    #expect(retry.delay(attempt: 2, unitRandom: 1) == 4.8)
    #expect(retry.delay(attempt: 20, unitRandom: 1) == 10)
  }

  @Test("HTTP transport sends gzip protobuf and honors Retry-After")
  func httpProtobufTransport() async throws {
    let configuration = try ChillOTLPConfiguration(
      endpoint: URL(string: "http://localhost:4318/v1/logs")!,
      headers: ["Authorization": "Bearer transport-secret"],
      allowInsecureLocalhost: true
    )
    let sessionConfiguration = URLSessionConfiguration.ephemeral
    sessionConfiguration.protocolClasses = [ExportURLProtocol.self]
    let session = URLSession(configuration: sessionConfiguration)
    defer { session.invalidateAndCancel() }
    let transport = OTLPHTTPTransport(
      configuration: configuration,
      session: session
    )
    let captured = OSAllocatedUnfairLock<URLRequest?>(initialState: nil)
    let capturedBody = OSAllocatedUnfairLock<Data?>(initialState: nil)
    ExportURLProtocol.handler = { request in
      captured.withLock { $0 = request }
      capturedBody.withLock { $0 = requestBody(request) }
      return ExportStubResponse(statusCode: 200, headers: [:])
    }
    let body = Data(repeating: 0x2A, count: 20_000)
    let export = OTLPExportRequest(recordIDs: ["record-1"], body: body)

    #expect(await transport.export(export) == .acknowledged(["record-1"]))
    let sent = try #require(captured.withLock { $0 })
    #expect(
      sent.value(forHTTPHeaderField: "Content-Type")
        == "application/x-protobuf"
    )
    #expect(sent.value(forHTTPHeaderField: "Content-Encoding") == "gzip")
    let compressedBody = try #require(capturedBody.withLock { $0 })
    #expect(
      GzipCompression.decompress(
        compressedBody,
        expectedByteCount: body.count
      ) == body
    )
    #expect(compressedBody.range(of: Data("transport-secret".utf8)) == nil)

    ExportURLProtocol.handler = { _ in
      ExportStubResponse(statusCode: 429, headers: ["Retry-After": "12"])
    }
    #expect(await transport.export(export) == .retry(after: 12))
  }

  private func decodeAttributes(
    _ attributes: [[String: Any]]
  ) -> [String: Any] {
    Dictionary(
      uniqueKeysWithValues: attributes.compactMap { item in
        guard let key = item["key"] as? String,
          let value = item["value"] as? [String: Any]
        else { return nil }
        return (key, value.values.first!)
      })
  }
}
@Test("Exporter source identity persists per queue and rotates per process")
func exporterSourceIdentityPersists() throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }

  let first = try ChillSourceMetadata.persistent(in: directory)
  let second = try ChillSourceMetadata.persistent(in: directory)

  #expect(first.platform == "apple")
  #expect(first.installationID == second.installationID)
  #expect(first.processID != second.processID)
  #expect(first.installationID.lowercased() == first.installationID)
  #expect(Array(first.installationID.utf8)[14] == 52)
}
