@_spi(TraceIntegration) import ChillCore
import ChillExport
import ChillNetworking
import ChillReplay
import Foundation

private enum RunnerError: Error, CustomStringConvertible {
  case usage
  case malformed(String)
  case unsupported(String)

  var description: String {
    switch self {
    case .usage:
      "usage: ChillConformanceRunner <manifest.json> [profile]"
    case .malformed(let detail):
      "malformed conformance fixture: \(detail)"
    case .unsupported(let domain):
      "unsupported conformance domain: \(domain)"
    }
  }
}

private typealias JSONObject = [String: Any]

private func object(_ value: Any?, _ label: String) throws -> JSONObject {
  guard let value = value as? JSONObject else {
    throw RunnerError.malformed(label)
  }
  return value
}

private func objects(_ value: Any?, _ label: String) throws -> [JSONObject] {
  guard let value = value as? [JSONObject] else {
    throw RunnerError.malformed(label)
  }
  return value
}

private func strings(_ value: Any?, _ label: String) throws -> [String] {
  guard let value = value as? [String] else {
    throw RunnerError.malformed(label)
  }
  return value
}

private func string(_ value: Any?, _ label: String) throws -> String {
  guard let value = value as? String else {
    throw RunnerError.malformed(label)
  }
  return value
}

private func integer(_ value: Any?, _ label: String) throws -> Int {
  guard let value = value as? NSNumber else {
    throw RunnerError.malformed(label)
  }
  return value.intValue
}

private func boolean(_ value: Any?, _ label: String) throws -> Bool {
  guard let value = value as? NSNumber else {
    throw RunnerError.malformed(label)
  }
  return value.boolValue
}

private func url(_ value: Any?, _ label: String) throws -> URL {
  let rawValue = try string(value, label)
  guard let value = URL(string: rawValue) else {
    throw RunnerError.malformed(label)
  }
  return value
}

private func loadJSON(_ url: URL) throws -> JSONObject {
  try object(
    JSONSerialization.jsonObject(with: Data(contentsOf: url)),
    url.path
  )
}

private func resolvedScenarioIDs(
  profile: String,
  profiles: JSONObject
) throws -> [String] {
  let entry = try object(profiles[profile], "profile \(profile)")
  var result: [String] = []
  if let inherited = entry["extends"] as? String {
    result.append(
      contentsOf: try resolvedScenarioIDs(
        profile: inherited,
        profiles: profiles
      ))
  }
  for id in try strings(entry["scenarios"], "profile scenarios")
  where !result.contains(id) {
    result.append(id)
  }
  return result
}

private func annotationJSON(_ value: AnnotationValue) -> Any {
  switch value {
  case .string(let value): value
  case .boolean(let value): value
  case .integer(let value): value
  case .double(let value): value
  case .strings(let value): value
  case .booleans(let value): value
  case .integers(let value): value
  case .doubles(let value): value
  }
}

private func annotationValue(_ value: Any) throws -> AnnotationValue {
  if let value = value as? String { return .string(value) }
  if let value = value as? [String] { return .strings(value) }
  if let value = value as? NSNumber {
    if CFGetTypeID(value) == CFBooleanGetTypeID() {
      return .boolean(value.boolValue)
    }
    return .integer(value.int64Value)
  }
  throw RunnerError.malformed("annotation value")
}

private func originJSON(_ origin: AnnotationOrigin) -> JSONObject {
  [
    "scope_id": origin.scopeID.rawValue,
    "depth": origin.depth,
    "declaration_index": origin.declarationIndex,
  ]
}

private func annotationsScenario(_ given: JSONObject) throws -> JSONObject {
  var context = AnnotationContext()
  for scope in try objects(given["scopes"], "annotation scopes") {
    let declarations = try objects(
      scope["declarations"],
      "annotation declarations"
    ).map { item in
      try AnnotationDeclaration(
        name: AnnotationName(try string(item["key"], "annotation key")),
        value: try annotationValue(item["value"] as Any)
      )
    }
    context = try context.addingScope(
      id: AnnotationScopeID(try string(scope["scope_id"], "scope id")),
      declarations: declarations
    )
  }
  let snapshot = context.snapshot
  return [
    "values": Dictionary(
      uniqueKeysWithValues: snapshot.values.map {
        ($0.key.rawValue, annotationJSON($0.value))
      }),
    "origins": Dictionary(
      uniqueKeysWithValues: snapshot.origins.map {
        ($0.key.rawValue, originJSON($0.value))
      }),
    "collisions": snapshot.collisions.map { collision in
      [
        "key": collision.name.rawValue,
        "winner": originJSON(collision.winner),
        "loser": originJSON(collision.loser),
        "identical": collision.identical,
      ] as JSONObject
    },
  ]
}

private func relation(_ rawValue: String) throws -> PageRelation {
  guard let value = PageRelation(rawValue: rawValue) else {
    throw RunnerError.malformed("page relation")
  }
  return value
}

private func pathJSON(_ path: PagePath) -> JSONObject {
  [
    "surface_id": path.surfaceID.rawValue,
    "instance_id": path.instanceID.rawValue,
    "instance_ids": path.instanceIDs.map(\.rawValue),
    "segments": path.segments.map(\.rawValue),
    "relation": path.relation.rawValue,
    "exposure": path.exposure.rawValue,
    "focused": path.focused,
  ]
}

private func makePath(
  surfaceID: String,
  instanceIDs: [String],
  segments: [String],
  relation: PageRelation,
  exposure: PageExposure,
  focused: Bool
) throws -> PagePath {
  guard let lastInstanceID = instanceIDs.last else {
    throw RunnerError.malformed("empty page path")
  }
  return try PagePath(
    surfaceID: SurfaceID(surfaceID),
    instanceID: PageInstanceID(lastInstanceID),
    instanceIDs: instanceIDs.map { try PageInstanceID($0) },
    segments: segments.map { try PageSegment($0) },
    relation: relation,
    exposure: exposure,
    focused: focused
  )
}

private func transitionJSON(
  operation: String,
  started: [String],
  updated: [String],
  ended: [String],
  paths: [PagePath]
) -> JSONObject {
  [
    "operation": operation,
    "delta": [
      "started": started,
      "updated": updated,
      "ended": ended,
    ],
    "exposed_paths": paths.map(pathJSON),
  ]
}

private func navigationScenario(_ given: JSONObject) throws -> JSONObject {
  let surfaceID = try string(given["surface_id"], "surface id")
  let instanceIDs = try strings(given["instance_ids"], "instance ids")
  let linear = try objects(given["linear_path"], "linear path")
  guard linear.count == 3, instanceIDs.count == 4 else {
    throw RunnerError.malformed("navigation fixture shape")
  }
  let segments = try linear.map { try string($0["segment"], "segment") }
  let cat = try makePath(
    surfaceID: surfaceID,
    instanceIDs: Array(instanceIDs.prefix(3)),
    segments: segments,
    relation: try relation(try string(linear[2]["relation"], "relation")),
    exposure: .foreground,
    focused: true
  )
  let presentation = try object(given["presentation"], "presentation")
  let visibleCat = try makePath(
    surfaceID: surfaceID,
    instanceIDs: Array(instanceIDs.prefix(3)),
    segments: segments,
    relation: cat.relation,
    exposure: .visible,
    focused: false
  )
  let sheet = try makePath(
    surfaceID: surfaceID,
    instanceIDs: instanceIDs,
    segments: segments + [try string(presentation["segment"], "segment")],
    relation: try relation(try string(presentation["relation"], "relation")),
    exposure: .foreground,
    focused: true
  )
  return [
    "transitions": [
      transitionJSON(
        operation: "reconcile",
        started: Array(instanceIDs.prefix(3)),
        updated: [],
        ended: [],
        paths: [cat]
      ),
      transitionJSON(
        operation: "present",
        started: [instanceIDs[3]],
        updated: [instanceIDs[2]],
        ended: [],
        paths: [visibleCat, sheet]
      ),
      transitionJSON(
        operation: "dismiss",
        started: [],
        updated: [instanceIDs[2]],
        ended: [instanceIDs[3]],
        paths: [cat]
      ),
    ],
    "primary_paths": [pathJSON(cat)],
  ]
}

private func actionsScenario(_ given: JSONObject) throws -> JSONObject {
  var seen: Set<String> = []
  var keys: [[String]] = []
  var facts: [JSONObject] = []
  var diagnostics: [JSONObject] = []
  for (index, item) in try objects(given["observations"], "observations")
    .enumerated()
  {
    let surface = try string(item["surface_id"], "surface id")
    let activationID = try string(
      item["native_activation_id"],
      "native activation id"
    )
    let key = "\(surface)\0\(activationID)"
    if !seen.insert(key).inserted {
      diagnostics.append([
        "observation_index": index,
        "code": "action.duplicate_observation",
      ])
      continue
    }
    keys.append([surface, activationID])
    let name = try SemanticName(try string(item["name"], "action name"))
    let role = try SemanticName(try string(item["role"], "action role"))
    _ = try ElementIdentity(
      surfaceID: SurfaceID(surface),
      instanceID: ElementInstanceID(
        try string(item["element_instance_id"], "element instance id")
      ),
      name: name,
      role: role
    )
    guard
      ActionActivation(
        rawValue: try string(item["activation"], "activation")
      ) != nil,
      InputKind(rawValue: try string(item["input_kind"], "input kind")) != nil
    else {
      throw RunnerError.malformed("action enums")
    }
    facts.append([
      "record_id": try string(item["record_id"], "record id"),
      "name": name.rawValue,
      "role": role.rawValue,
      "activation": try string(item["activation"], "activation"),
      "input_kind": try string(item["input_kind"], "input kind"),
      "surface_id": surface,
      "element_instance_id": try string(
        item["element_instance_id"],
        "element instance id"
      ),
    ])
  }
  return [
    "facts": facts,
    "diagnostics": diagnostics,
    "deduplication_keys": keys,
  ]
}

private func context(_ raw: JSONObject, remote: Bool? = nil) throws -> TraceContext {
  try TraceContext(
    traceID: string(raw["trace_id"], "trace id"),
    spanID: string(raw["span_id"], "span id"),
    traceFlags: UInt8(integer(raw["trace_flags"] ?? 0, "trace flags")),
    traceState: raw["trace_state"] as? String,
    isRemote: remote ?? ((raw["remote"] as? NSNumber)?.boolValue ?? false)
  )
}

private func contextJSON(_ context: TraceContext?) -> Any {
  guard let context else { return NSNull() }
  return [
    "trace_id": context.traceID,
    "span_id": context.spanID,
    "trace_flags": Int(context.traceFlags),
    "trace_state": context.traceState ?? NSNull(),
    "remote": context.isRemote,
  ] as JSONObject
}

private func traceScenario(_ given: JSONObject) throws -> JSONObject {
  let span = try context(try object(given["context"], "context"))
  let policyValue = try object(given["policy"], "policy")
  var baggage: [String: String] = [:]
  for (key, value) in try object(given["baggage"], "baggage") {
    baggage[key] = try string(value, "baggage value")
  }
  let policy = try ChillNetworkPropagationPolicy(
    trustedOriginURLs: try strings(
      policyValue["trusted_origins"],
      "trusted origins"
    ).map { try url($0, "trusted origin") },
    baggageAllowlist: Set(
      try strings(policyValue["baggage_allowlist"], "baggage allowlist")
    ),
    baggage: baggage
  )
  var headers: JSONObject = [:]
  for destination in try strings(given["destinations"], "destinations") {
    let prepared = NetworkHeaderPropagation.prepare(
      URLRequest(url: try url(destination, "destination")),
      context: span,
      policy: policy
    )
    var values: JSONObject = [:]
    for name in ["traceparent", "tracestate", "baggage"] {
      if let value = prepared.value(forHTTPHeaderField: name) {
        values[name] = value
      }
    }
    headers[destination] = values
  }
  let messaging = try object(given["messaging"], "messaging")
  let ambient = try context(try object(messaging["ambient"], "ambient"))
  let candidateLinks = try objects(
    messaging["creation_contexts"],
    "creation contexts"
  ).map { try context($0) }
  var seenLinks: Set<String> = []
  let links = candidateLinks.filter {
    seenLinks.insert("\($0.traceID)\0\($0.spanID)").inserted
  }
  return [
    "http_headers": headers,
    "parsed_valid": contextJSON(
      W3CTraceContext.extract(
        traceParent: try string(given["valid_traceparent"], "traceparent")
      )
    ),
    "parsed_invalid": contextJSON(
      W3CTraceContext.extract(
        traceParent: try string(given["invalid_traceparent"], "traceparent")
      )
    ),
    "messaging": [
      "parent": contextJSON(ambient),
      "links": links.map(contextJSON),
    ],
  ]
}

private func offlineScenario(_ given: JSONObject) throws -> JSONObject {
  struct Entry {
    let recordID: String
    let kind: String
    let priority: String
    let bytes: Int
    let sequence: Int
  }
  let priority: [String: ChillQueuePriority] = [
    "replay": .replay,
    "low": .low,
    "high": .high,
  ]
  let capacity = try integer(given["capacity_bytes"], "capacity")
  var queue: [Entry] = []
  var dropped: [JSONObject] = []
  var attempts: [[String]] = []
  var recoveries = 0
  var sequence = 0
  for step in try objects(given["steps"], "offline steps") {
    switch try string(step["operation"], "offline operation") {
    case "enqueue":
      sequence += 1
      let priorityName = try string(step["priority"], "priority")
      guard priority[priorityName] != nil else {
        throw RunnerError.malformed("priority")
      }
      queue.append(
        Entry(
          recordID: try string(step["record_id"], "record id"),
          kind: try string(step["kind"], "kind"),
          priority: priorityName,
          bytes: try integer(step["bytes"], "bytes"),
          sequence: sequence
        ))
      while queue.reduce(0, { $0 + $1.bytes }) > capacity {
        guard
          let victim = queue.indices.min(by: { left, right in
            let leftPriority = priority[queue[left].priority]?.rawValue ?? 0
            let rightPriority = priority[queue[right].priority]?.rawValue ?? 0
            return leftPriority == rightPriority
              ? queue[left].sequence < queue[right].sequence
              : leftPriority < rightPriority
          })
        else {
          throw RunnerError.malformed("empty over-capacity queue")
        }
        let removed = queue.remove(at: victim)
        dropped.append([
          "record_id": removed.recordID,
          "kind": removed.kind,
          "reason": "capacity",
        ])
      }
    case "crash_recover":
      recoveries += 1
    case "export":
      let batch = Array(
        queue.prefix(
          try integer(step["max_records"], "max records")
        ))
      attempts.append(batch.map(\.recordID))
      let result = try string(step["result"], "export result")
      if result == "ack" {
        let acknowledged = Set(
          try strings(
            step["acknowledged_record_ids"],
            "acknowledged ids"
          ))
        guard acknowledged.isSubset(of: Set(batch.map(\.recordID))) else {
          throw RunnerError.malformed("acknowledged batch")
        }
        queue.removeAll { acknowledged.contains($0.recordID) }
      } else if result != "failure" {
        throw RunnerError.malformed("export result")
      }
    default:
      throw RunnerError.malformed("offline operation")
    }
  }
  return [
    "queued": queue.map {
      [
        "record_id": $0.recordID,
        "kind": $0.kind,
        "priority": $0.priority,
        "bytes": $0.bytes,
      ] as JSONObject
    },
    "dropped": dropped,
    "export_attempts": attempts,
    "recovery_count": recoveries,
    "queued_bytes": queue.reduce(0, { $0 + $1.bytes }),
  ]
}

private func replayScenario(_ given: JSONObject) throws -> JSONObject {
  let chunks = try objects(given["chunks"], "chunks")
  var aligned: [JSONObject] = []
  for fact in try objects(given["facts"], "facts") {
    let bootID = try string(fact["boot_id"], "boot id")
    let moment = try integer(fact["monotonic_nano"], "monotonic time")
    let candidates = try chunks.filter { chunk in
      try string(chunk["boot_id"], "boot id") == bootID
        && integer(chunk["start_monotonic_nano"], "chunk start") <= moment
        && moment < integer(chunk["end_monotonic_nano"], "chunk end")
    }
    guard candidates.count == 1 else {
      throw RunnerError.malformed("replay alignment")
    }
    let chunk = candidates[0]
    aligned.append([
      "record_id": try string(fact["record_id"], "record id"),
      "chunk_id": try string(chunk["chunk_id"], "chunk id"),
      "offset_nano": moment
        - (try integer(chunk["start_monotonic_nano"], "chunk start")),
    ])
  }
  return ["aligned_facts": aligned, "clock_basis": "boot_monotonic"]
}

private func schemaScenario(_ given: JSONObject) throws -> JSONObject {
  let supported = try object(given["supported"], "supported schemas")
  let versionPattern = try NSRegularExpression(
    pattern: #"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$"#
  )
  let supportedMajors = Set(
    supported.keys.compactMap {
      $0.split(separator: ".").first.flatMap { Int($0) }
    })
  let results: [JSONObject] = try objects(
    given["requests"],
    "schema requests"
  ).map { request in
    let version = try string(request["schema_version"], "schema version")
    let url = try string(request["schema_url"], "schema url")
    let range = NSRange(version.startIndex..., in: version)
    let accepted: Bool
    let reason: String
    if versionPattern.firstMatch(in: version, range: range) == nil {
      (accepted, reason) = (false, "invalid_version")
    } else if supported[version] == nil {
      guard let component = version.split(separator: ".").first,
        let major = Int(component)
      else {
        throw RunnerError.malformed("schema major")
      }
      (accepted, reason) = (
        false,
        supportedMajors.contains(major)
          ? "unsupported_version" : "unsupported_major"
      )
    } else if (supported[version] as? String) != url {
      (accepted, reason) = (false, "schema_url_mismatch")
    } else {
      (accepted, reason) = (true, "accepted_exact")
    }
    return [
      "case": try string(request["case"], "schema case"),
      "accepted": accepted,
      "reason": reason,
      "negotiated_version": accepted ? version : NSNull(),
    ]
  }
  return ["results": results]
}

private func maskJSON(_ mask: ReplayContentMask) -> JSONObject {
  var result: JSONObject = ["kind": mask.kind.rawValue]
  if let bucket = mask.lengthBucket {
    result["length_bucket"] = bucket.rawValue
  }
  return result
}

private func redactionScenario(_ given: JSONObject) throws -> JSONObject {
  var records: [JSONObject] = []
  for item in try objects(given["inputs"], "redaction inputs") {
    switch try string(item["kind"], "redaction kind") {
    case "ui":
      guard let geometry = item["geometry"] as? [NSNumber],
        geometry.count == 4
      else {
        throw RunnerError.malformed("UI geometry")
      }
      _ = ReplayRect(
        x: geometry[0].doubleValue,
        y: geometry[1].doubleValue,
        width: geometry[2].doubleValue,
        height: geometry[3].doubleValue
      )
      let mask: ReplayContentMask
      if (item["secure"] as? NSNumber)?.boolValue == true {
        mask = .secureInput
      } else if let text = item["text"] as? String {
        mask = .text(length: text.count)
      } else if (item["pixels"] as? NSNumber)?.boolValue == true {
        mask = .pixels
      } else {
        throw RunnerError.malformed("UI redaction mask")
      }
      records.append([
        "kind": "ui",
        "element_id": try string(item["element_id"], "element id"),
        "role": try string(item["role"], "role"),
        "geometry": geometry,
        "content_mask": maskJSON(mask),
      ])
    case "http":
      records.append([
        "kind": "http",
        "method": try string(item["method"], "method"),
        "route_template": try string(
          item["route_template"],
          "route template"
        ),
        "status_code": try integer(item["status_code"], "status code"),
      ])
    case "error":
      records.append([
        "kind": "error",
        "reason_code": try string(item["reason_code"], "reason code"),
      ])
    default:
      throw RunnerError.malformed("redaction input kind")
    }
  }
  return ["records": records, "redaction_stage": "before_buffer"]
}

private func samplingScenario(_ given: JSONObject) throws -> JSONObject {
  let sampler = try DeterministicSampler(
    salt: string(given["salt"], "sampling salt")
  )
  let streamValues = try object(given["streams"], "sampling streams")
  let results: [JSONObject] = try objects(
    given["cases"],
    "sampling cases"
  ).map { item in
    var decisions: JSONObject = [:]
    for stream in [SamplingStream.behavior, .replay] {
      let rawRate = try object(streamValues[stream.rawValue], "sampling rate")
      let rate = try SamplingRate(
        numerator: UInt64(integer(rawRate["numerator"], "numerator")),
        denominator: UInt64(integer(rawRate["denominator"], "denominator"))
      )
      let decision = sampler.decide(
        stream: stream,
        stableKey: try string(item["stable_key"], "stable key"),
        rate: rate
      )
      decisions[stream.rawValue] = [
        "kept": decision.kept,
        "digest_prefix": decision.digestPrefix,
      ]
    }
    return [
      "case": try string(item["case"], "sampling case"),
      "stable_key": try string(item["stable_key"], "stable key"),
      "trace_sampled": try boolean(item["trace_sampled"], "trace sampled"),
      "decisions": decisions,
    ]
  }
  return ["algorithm": "sha256-first-u64", "results": results]
}

private func output(domain: String, given: JSONObject) throws -> JSONObject {
  switch domain {
  case "annotations": try annotationsScenario(given)
  case "navigation": try navigationScenario(given)
  case "actions": try actionsScenario(given)
  case "trace": try traceScenario(given)
  case "offline": try offlineScenario(given)
  case "replay": try replayScenario(given)
  case "schema": try schemaScenario(given)
  case "redaction": try redactionScenario(given)
  case "sampling": try samplingScenario(given)
  default: throw RunnerError.unsupported(domain)
  }
}

private func run() throws {
  guard CommandLine.arguments.count >= 2 else { throw RunnerError.usage }
  let manifestURL = URL(fileURLWithPath: CommandLine.arguments[1])
  let profile =
    CommandLine.arguments.count >= 3
    ? CommandLine.arguments[2] : "client.replay"
  let manifest = try loadJSON(manifestURL)
  let scenarioIDs = try resolvedScenarioIDs(
    profile: profile,
    profiles: try object(manifest["profiles"], "profiles")
  )
  let descriptors = try objects(manifest["scenarios"], "scenarios")
  let byID = try Dictionary(
    uniqueKeysWithValues: descriptors.map {
      (try string($0["id"], "scenario id"), $0)
    })
  var results: [JSONObject] = []
  for scenarioID in scenarioIDs {
    guard let descriptor = byID[scenarioID] else {
      throw RunnerError.malformed("scenario descriptor \(scenarioID)")
    }
    let path = try string(descriptor["path"], "scenario path")
    let scenario = try loadJSON(
      manifestURL.deletingLastPathComponent().appendingPathComponent(path)
    )
    results.append([
      "scenario_id": scenarioID,
      "status": "passed",
      "output": try output(
        domain: try string(scenario["domain"], "scenario domain"),
        given: try object(scenario["given"], "scenario given")
      ),
    ])
  }
  let encoded = try JSONSerialization.data(
    withJSONObject: ["profile": profile, "results": results],
    options: [.prettyPrinted, .sortedKeys]
  )
  print(String(decoding: encoded, as: UTF8.self))
}

do {
  try run()
} catch {
  fputs("error: \(error)\n", stderr)
  exit(2)
}
