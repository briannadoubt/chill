import ChillCore
import Foundation

package enum OTLPJSONEncoder {
  private static let instrumentationName = "dev.chill.swift"
  private static let instrumentationVersion = "0.1.0"
  private static let semanticConventionVersion = "1.43.0"

  package static func encodeLogRecord(_ record: BehaviorRecord) throws -> Data {
    let attributes = projectedAttributes(for: record)
    var logRecord: [String: Any] = [
      "timeUnixNano": String(record.clock.occurredAtUnixNano),
      "observedTimeUnixNano": String(record.clock.observedAtUnixNano),
      "eventName": eventName(for: record),
      "attributes": encodeAttributes(attributes),
    ]
    if let trace = record.trace {
      logRecord["traceId"] = trace.traceID
      logRecord["spanId"] = trace.spanID
      logRecord["flags"] = Int(trace.traceFlags & 0x01)
    }
    if case .event(let payload) = record.payload {
      logRecord["severityNumber"] = severityNumber(payload.severity)
      logRecord["severityText"] = payload.severity.rawValue.uppercased()
    }
    return try JSONSerialization.data(
      withJSONObject: logRecord,
      options: [.sortedKeys]
    )
  }

  package static func projectedAttributes(
    for record: BehaviorRecord,
    sourceMetadata: ChillSourceMetadata? = nil
  ) -> [String: OTLPAnyValue] {
    var attributes: [String: OTLPAnyValue] = [
      "chill.schema.version": .string(record.schemaVersion),
      "chill.schema.url": .string(record.schemaURL),
      "chill.record.id": .string(record.recordID.rawValue),
      "chill.subject.id": .string(record.subjectID.rawValue),
      "chill.context.session_id": .string(record.sessionID.rawValue),
      "chill.behavior.kind": .string(record.kind.rawValue),
      "chill.behavior.operation": .string(record.operation.rawValue),
      "chill.behavior.name": .string(record.name.rawValue),
      "chill.clock.monotonic_nano": .integer(record.clock.monotonicNano),
      "chill.clock.boot_id": .string(record.clock.bootID.rawValue),
      "chill.clock.sequence_number": .integer(record.clock.sequenceNumber),
      "chill.privacy.capture_class": .string(record.captureClass.rawValue),
      "chill.privacy.consent": .string(record.consent.rawValue),
      "chill.privacy.policy_version": .string(record.policyVersion),
      "chill.privacy.redaction_state": .string(record.redactionState.rawValue),
      "chill.otel.semconv.version": .string(semanticConventionVersion),
      "session.id": .string(record.sessionID.rawValue),
    ]
    if let sourceMetadata {
      attributes["chill.source.platform"] = .string(sourceMetadata.platform)
      attributes["chill.source.installation_id"] = .string(
        sourceMetadata.installationID
      )
      attributes["chill.source.process_id"] = .string(
        sourceMetadata.processID
      )
      if let appBuild = sourceMetadata.appBuild {
        attributes["chill.source.app_build"] = .string(appBuild)
      }
    }
    if record.redactionCount > 0 {
      attributes["chill.privacy.redaction_count"] = .integer(
        UInt64(record.redactionCount)
      )
    }
    if let duration = record.durationNano {
      attributes["chill.duration_nano"] = .integer(duration)
    }
    for (name, classification) in record.annotations.classifications {
      guard let value = record.annotations.values[name],
        classification.isAnnotationEligible
      else { continue }
      attributes["chill.annotation.\(name.rawValue)"] = .annotation(value)
      attributes["chill.privacy.annotation_classification.\(name.rawValue)"] = .string(
        classification.rawValue
      )
    }
    appendPage(record.page, prefix: "chill.context.page", to: &attributes)
    if let page = record.page {
      attributes["app.screen.id"] = .string(page.instanceID.rawValue)
      if let segment = page.segments.last {
        attributes["app.screen.name"] = .string(segment.rawValue)
      }
    }
    if let trace = record.trace {
      if let parent = trace.parentSpanID {
        attributes["chill.trace.parent_span_id"] = .string(parent)
      }
      if let state = trace.traceState {
        attributes["chill.trace.trace_state"] = .string(state)
      }
    }
    appendPayload(record.payload, to: &attributes)
    return attributes
  }

  package static func encodeBatch(
    _ entries: [StoredQueueEntry],
    resourceAttributes: [String: String]
  ) throws -> Data {
    var resource = [
      "telemetry.sdk.name": "chill",
      "telemetry.sdk.language": "swift",
      "telemetry.sdk.version": instrumentationVersion,
    ]
    resource.merge(resourceAttributes) { current, _ in current }
    let logs = try entries.map {
      try JSONSerialization.jsonObject(with: $0.payload)
    }
    let request: [String: Any] = [
      "resourceLogs": [
        [
          "resource": [
            "attributes": encodeAttributes(
              resource.mapValues(OTLPAnyValue.string)
            )
          ],
          "scopeLogs": [
            [
              "scope": [
                "name": instrumentationName,
                "version": instrumentationVersion,
              ],
              "schemaUrl": "https://opentelemetry.io/schemas/1.43.0",
              "logRecords": logs,
            ]
          ],
        ]
      ]
    ]
    return try JSONSerialization.data(
      withJSONObject: request,
      options: [.sortedKeys]
    )
  }

  private static func appendPage(
    _ page: PagePath?,
    prefix: String,
    to attributes: inout [String: OTLPAnyValue]
  ) {
    guard let page else { return }
    attributes["\(prefix).surface_id"] = .string(page.surfaceID.rawValue)
    attributes["\(prefix).instance_id"] = .string(page.instanceID.rawValue)
    attributes["\(prefix).path_instance_ids"] = .strings(
      page.instanceIDs.map(\.rawValue)
    )
    attributes["\(prefix).path"] = .strings(page.segments.map(\.rawValue))
  }

  private static func appendPayload(
    _ payload: BehaviorPayload,
    to attributes: inout [String: OTLPAnyValue]
  ) {
    let prefix = "chill.payload"
    switch payload {
    case .none:
      break
    case .page(let value):
      appendPage(value.path, prefix: prefix, to: &attributes)
      attributes["\(prefix).relation"] = .string(value.path.relation.rawValue)
      attributes["\(prefix).exposure"] = .string(value.path.exposure.rawValue)
      attributes["\(prefix).focused"] = .boolean(value.path.focused)
      attributes["\(prefix).cause"] = .string(value.cause.rawValue)
      if let navigationID = value.navigationID {
        attributes["\(prefix).navigation_id"] = .string(navigationID.rawValue)
      }
      if let ratio = value.visibilityRatio {
        attributes["\(prefix).visibility_ratio"] = .double(ratio)
      }
    case .impression(let value):
      attributes["\(prefix).element_id"] = .string(value.elementID.rawValue)
      attributes["\(prefix).role"] = .string(value.role.rawValue)
      attributes["\(prefix).visibility_ratio"] = .double(
        value.visibilityRatio
      )
      if let duration = value.visibleDurationNano {
        attributes["\(prefix).visible_duration_nano"] = .integer(duration)
      }
    case .action(let value):
      attributes["\(prefix).element_id"] = .string(value.elementID.rawValue)
      attributes["\(prefix).role"] = .string(value.role.rawValue)
      attributes["\(prefix).activation"] = .string(value.activation.rawValue)
      attributes["\(prefix).input"] = .string(value.input.rawValue)
      attributes["app.widget.id"] = .string(value.elementID.rawValue)
    case .activity(let value):
      attributes["\(prefix).activity_kind"] = .string(value.kind.rawValue)
      attributes["\(prefix).role"] = .string(value.role.rawValue)
      attributes["\(prefix).attempt"] = .integer(UInt64(value.attempt))
      attributes["\(prefix).recursion_depth"] = .integer(
        UInt64(value.recursionDepth)
      )
      if let parent = value.parentActivityID {
        attributes["\(prefix).parent_activity_id"] = .string(parent.rawValue)
      }
      if let outcome = value.outcome {
        attributes["chill.outcome.status"] = .string(
          canonicalOutcome(outcome)
        )
      }
      if let reason = value.reasonCode {
        attributes["chill.outcome.reason_code"] = .string(reason.rawValue)
      }
    case .event(let value):
      attributes["\(prefix).event_class"] = .string(value.eventClass.rawValue)
      attributes["\(prefix).severity"] = .string(value.severity.rawValue)
      attributes["\(prefix).emission"] = .string(value.emission.rawValue)
      if let outcome = value.outcome {
        attributes["chill.outcome.status"] = .string(
          canonicalOutcome(outcome)
        )
      }
      if let key = value.deduplicationKeyHash {
        attributes["\(prefix).deduplication_key_hash"] = .string(key)
      }
    case .replay(let value):
      attributes["chill.context.replay_id"] = .string(value.replayID)
      attributes["\(prefix).chunk_id"] = .string(value.chunkID)
      attributes["\(prefix).chunk_index"] = .integer(UInt64(value.chunkIndex))
      attributes["\(prefix).starts_at_unix_nano"] = .integer(
        value.startsAtUnixNano
      )
      attributes["\(prefix).ends_at_unix_nano"] = .integer(
        value.endsAtUnixNano
      )
      attributes["\(prefix).sha256"] = .string(value.sha256)
      attributes["\(prefix).byte_count"] = .integer(value.byteCount)
      attributes["\(prefix).codec"] = .string("chill.structural.v1")
      attributes["\(prefix).storage_ref"] = .string(value.storageRef)
    }
  }

  package static func eventName(for record: BehaviorRecord) -> String {
    if case .action(let payload) = record.payload {
      if payload.activation == .primary,
        payload.input == .touch || payload.input == .pointer
      {
        return "app.widget.click"
      }
      return "chill.action"
    }
    switch record.kind {
    case .event, .impression, .replay:
      return "chill.\(record.kind.rawValue)"
    default:
      return "chill.\(record.kind.rawValue).\(record.operation.rawValue)"
    }
  }

  package static func severityNumber(_ severity: EventSeverity) -> Int {
    switch severity {
    case .trace: 1
    case .debug: 5
    case .info: 9
    case .warn: 13
    case .error: 17
    case .fatal: 21
    }
  }

  private static func canonicalOutcome(_ outcome: TerminalOutcome) -> String {
    switch outcome {
    case .succeeded: "ok"
    case .failed: "error"
    case .cancelled: "cancelled"
    case .timedOut: "timeout"
    }
  }

  private static func encodeAttributes(
    _ attributes: [String: OTLPAnyValue]
  ) -> [[String: Any]] {
    attributes.keys.sorted().map { key in
      ["key": key, "value": attributes[key]!.json]
    }
  }
}

package enum OTLPAnyValue: Sendable {
  case string(String)
  case boolean(Bool)
  case integer(UInt64)
  case signedInteger(Int64)
  case double(Double)
  case strings([String])
  case booleans([Bool])
  case integers([Int64])
  case doubles([Double])

  package static func annotation(_ value: AnnotationValue) -> OTLPAnyValue {
    switch value {
    case .string(let value): .string(value)
    case .boolean(let value): .boolean(value)
    case .integer(let value): .signedInteger(value)
    case .double(let value): .double(value)
    case .strings(let value): .strings(value)
    case .booleans(let value): .booleans(value)
    case .integers(let value): .integers(value)
    case .doubles(let value): .doubles(value)
    }
  }

  var json: [String: Any] {
    switch self {
    case .string(let value): ["stringValue": value]
    case .boolean(let value): ["boolValue": value]
    case .integer(let value): ["intValue": String(value)]
    case .signedInteger(let value): ["intValue": String(value)]
    case .double(let value): ["doubleValue": value]
    case .strings(let values): array(values.map { .string($0) })
    case .booleans(let values): array(values.map { .boolean($0) })
    case .integers(let values): array(values.map { .signedInteger($0) })
    case .doubles(let values): array(values.map { .double($0) })
    }
  }

  private func array(_ values: [OTLPAnyValue]) -> [String: Any] {
    ["arrayValue": ["values": values.map(\.json)]]
  }
}
