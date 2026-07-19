import ChillCore
import Foundation

/// Dependency-free encoder for the stable OTLP Logs protobuf messages. Keeping
/// this small avoids shipping a second general-purpose telemetry SDK merely to
/// write the dedicated behavior queue.
package enum OTLPProtobufEncoder {
  private static let instrumentationName = "dev.chill.swift"
  private static let instrumentationVersion = "0.1.0"
  private static let schemaURL = "https://opentelemetry.io/schemas/1.43.0"

  package static func encodeLogRecord(
    _ record: BehaviorRecord,
    sourceMetadata: ChillSourceMetadata? = nil
  ) -> Data {
    var writer = ProtobufWriter()
    writer.fixed64(field: 1, value: record.clock.occurredAtUnixNano)
    if case .event(let payload) = record.payload {
      writer.varint(
        field: 2,
        value: UInt64(OTLPJSONEncoder.severityNumber(payload.severity))
      )
      writer.string(field: 3, value: payload.severity.rawValue.uppercased())
    }
    let attributes = OTLPJSONEncoder.projectedAttributes(
      for: record,
      sourceMetadata: sourceMetadata
    )
    for key in attributes.keys.sorted() {
      writer.message(
        field: 6,
        value: encodeKeyValue(key: key, value: attributes[key]!)
      )
    }
    if let trace = record.trace {
      writer.fixed32(field: 8, value: UInt32(trace.traceFlags & 0x01))
      if let traceID = decodeHex(trace.traceID) {
        writer.bytes(field: 9, value: traceID)
      }
      if let spanID = decodeHex(trace.spanID) {
        writer.bytes(field: 10, value: spanID)
      }
    }
    writer.fixed64(field: 11, value: record.clock.observedAtUnixNano)
    writer.string(field: 12, value: OTLPJSONEncoder.eventName(for: record))
    return writer.data
  }

  package static func encodeBatch(
    _ entries: [StoredQueueEntry],
    resourceAttributes: [String: String]
  ) -> Data {
    var resourceValues = [
      "telemetry.sdk.name": "chill",
      "telemetry.sdk.language": "swift",
      "telemetry.sdk.version": instrumentationVersion,
    ]
    resourceValues.merge(resourceAttributes) { current, _ in current }

    var resource = ProtobufWriter()
    for key in resourceValues.keys.sorted() {
      resource.message(
        field: 1,
        value: encodeKeyValue(
          key: key,
          value: .string(resourceValues[key]!)
        )
      )
    }

    var scope = ProtobufWriter()
    scope.string(field: 1, value: instrumentationName)
    scope.string(field: 2, value: instrumentationVersion)

    var scopeLogs = ProtobufWriter()
    scopeLogs.message(field: 1, value: scope.data)
    for entry in entries {
      scopeLogs.message(field: 2, value: entry.payload)
    }
    scopeLogs.string(field: 3, value: schemaURL)

    var resourceLogs = ProtobufWriter()
    resourceLogs.message(field: 1, value: resource.data)
    resourceLogs.message(field: 2, value: scopeLogs.data)

    var request = ProtobufWriter()
    request.message(field: 1, value: resourceLogs.data)
    return request.data
  }

  private static func encodeKeyValue(
    key: String,
    value: OTLPAnyValue
  ) -> Data {
    var writer = ProtobufWriter()
    writer.string(field: 1, value: key)
    writer.message(field: 2, value: encodeAnyValue(value))
    return writer.data
  }

  private static func encodeAnyValue(_ value: OTLPAnyValue) -> Data {
    var writer = ProtobufWriter()
    switch value {
    case .string(let value):
      writer.string(field: 1, value: value)
    case .boolean(let value):
      writer.varint(field: 2, value: value ? 1 : 0)
    case .integer(let value):
      writer.varint(field: 3, value: value)
    case .signedInteger(let value):
      writer.varint(field: 3, value: UInt64(bitPattern: value))
    case .double(let value):
      writer.fixed64(field: 4, value: value.bitPattern)
    case .strings(let values):
      writer.message(field: 5, value: encodeArray(values.map(OTLPAnyValue.string)))
    case .booleans(let values):
      writer.message(field: 5, value: encodeArray(values.map(OTLPAnyValue.boolean)))
    case .integers(let values):
      writer.message(
        field: 5,
        value: encodeArray(values.map(OTLPAnyValue.signedInteger))
      )
    case .doubles(let values):
      writer.message(field: 5, value: encodeArray(values.map(OTLPAnyValue.double)))
    }
    return writer.data
  }

  private static func encodeArray(_ values: [OTLPAnyValue]) -> Data {
    var writer = ProtobufWriter()
    for value in values {
      writer.message(field: 1, value: encodeAnyValue(value))
    }
    return writer.data
  }

  private static func decodeHex(_ value: String) -> Data? {
    guard value.utf8.count.isMultiple(of: 2) else { return nil }
    let bytes = Array(value.utf8)
    var result = Data(capacity: bytes.count / 2)
    for index in stride(from: 0, to: bytes.count, by: 2) {
      guard let high = hexNibble(bytes[index]),
        let low = hexNibble(bytes[index + 1])
      else { return nil }
      result.append((high << 4) | low)
    }
    return result
  }

  private static func hexNibble(_ byte: UInt8) -> UInt8? {
    switch byte {
    case 48...57: byte - 48
    case 97...102: byte - 87
    default: nil
    }
  }
}

private struct ProtobufWriter {
  private(set) var data = Data()

  mutating func varint(field: Int, value: UInt64) {
    tag(field: field, wireType: 0)
    rawVarint(value)
  }

  mutating func fixed64(field: Int, value: UInt64) {
    tag(field: field, wireType: 1)
    for shift in stride(from: 0, through: 56, by: 8) {
      data.append(UInt8((value >> UInt64(shift)) & 0xFF))
    }
  }

  mutating func string(field: Int, value: String) {
    bytes(field: field, value: Data(value.utf8))
  }

  mutating func message(field: Int, value: Data) {
    bytes(field: field, value: value)
  }

  mutating func bytes(field: Int, value: Data) {
    tag(field: field, wireType: 2)
    rawVarint(UInt64(value.count))
    data.append(value)
  }

  mutating func fixed32(field: Int, value: UInt32) {
    tag(field: field, wireType: 5)
    for shift in stride(from: 0, through: 24, by: 8) {
      data.append(UInt8((value >> UInt32(shift)) & 0xFF))
    }
  }

  private mutating func tag(field: Int, wireType: UInt64) {
    rawVarint((UInt64(field) << 3) | wireType)
  }

  private mutating func rawVarint(_ original: UInt64) {
    var value = original
    while value >= 0x80 {
      data.append(UInt8(value & 0x7F) | 0x80)
      value >>= 7
    }
    data.append(UInt8(value))
  }
}
