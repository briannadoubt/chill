import ChillCore
import Foundation

package actor ReplayEngine {
  private let configuration: ChillReplayConfiguration
  private let store: ReplayChunkStore
  private var currentSessionID: String?
  private var currentBootID: String?
  private var replaySessionID: String?
  private var replayID: String?
  private var nextChunkIndex: UInt32 = 0
  private var currentChunkIndex: UInt32 = 0
  private var currentNodes: [String: ReplayNode] = [:]
  private var currentViewport: ReplayViewport?
  private var frames: [ReplayFrame] = []
  private var chunkID: String?
  private var chunkOccurredAtUnixNano: UInt64 = 0
  private var chunkStartMonotonicNano: UInt64 = 0
  private var lastMonotonicNano: UInt64 = 0
  private var emittedFrames: UInt64 = 0
  private var coalescedFrames: UInt64 = 0
  private var suppressedFrames: UInt64 = 0
  private var redactedNodes: UInt64 = 0
  private var evictedChunks: UInt64 = 0
  private var consentPurgeRequired = false

  package init(
    configuration: ChillReplayConfiguration
  ) throws {
    self.configuration = configuration
    store = try ReplayChunkStore(configuration: configuration)
    if let latest = store.descriptors().last {
      replaySessionID = latest.sessionID
      replayID = latest.replayID
      nextChunkIndex = latest.chunkIndex &+ 1
    }
  }

  package func ingest(_ observation: ReplayObservation) async {
    guard !consentPurgeRequired else {
      suppressedFrames &+= 1
      return
    }
    if !frames.isEmpty,
      currentSessionID != observation.sessionID
        || currentBootID != observation.bootID
    {
      await seal(endMonotonicNano: lastMonotonicNano &+ 1)
    }
    guard observation.monotonicNano >= lastMonotonicNano else {
      suppressedFrames &+= 1
      return
    }
    let boundedNodes = Array(observation.nodes.prefix(configuration.maximumNodes))
    if shouldRotate(at: observation.monotonicNano) {
      await seal(endMonotonicNano: observation.monotonicNano)
    }
    if frames.isEmpty {
      startChunk(
        occurredAtUnixNano: observation.occurredAtUnixNano,
        monotonicNano: observation.monotonicNano,
        sessionID: observation.sessionID,
        bootID: observation.bootID,
        viewport: observation.viewport,
        nodes: boundedNodes
      )
      return
    }

    let nextNodes = Self.indexed(boundedNodes)
    let upserted = boundedNodes.filter { currentNodes[$0.id] != $0 }
    let removed = currentNodes.keys.filter { nextNodes[$0] == nil }.sorted()
    let viewport = currentViewport == observation.viewport ? nil : observation.viewport
    guard !upserted.isEmpty || !removed.isEmpty || viewport != nil else {
      coalescedFrames &+= 1
      lastMonotonicNano = observation.monotonicNano
      return
    }
    let frame = ReplayFrame(
      kind: .delta,
      offsetNano: observation.monotonicNano - chunkStartMonotonicNano,
      viewport: viewport,
      upsertedNodes: upserted,
      removedNodeIDs: removed,
      gesture: nil
    )
    if estimatedMemory(afterAdding: frame) > configuration.maximumMemoryBytes {
      await seal(endMonotonicNano: observation.monotonicNano)
      startChunk(
        occurredAtUnixNano: observation.occurredAtUnixNano,
        monotonicNano: observation.monotonicNano,
        sessionID: observation.sessionID,
        bootID: observation.bootID,
        viewport: observation.viewport,
        nodes: boundedNodes
      )
      return
    }
    frames.append(frame)
    currentNodes = nextNodes
    currentViewport = observation.viewport
    lastMonotonicNano = observation.monotonicNano
    emittedFrames &+= 1
    redactedNodes &+= UInt64(
      upserted.lazy.filter {
        $0.contentMask != nil
      }.count)
  }

  package func ingest(
    gesture: ReplayGesture,
    occurredAtUnixNano: UInt64,
    monotonicNano: UInt64
  ) async {
    guard !consentPurgeRequired else {
      suppressedFrames &+= 1
      return
    }
    guard monotonicNano >= lastMonotonicNano else {
      suppressedFrames &+= 1
      return
    }
    if shouldRotate(at: monotonicNano) {
      await seal(endMonotonicNano: monotonicNano)
    }
    guard !frames.isEmpty, currentViewport != nil else {
      suppressedFrames &+= 1
      return
    }
    let frame = ReplayFrame(
      kind: .gesture,
      offsetNano: monotonicNano - chunkStartMonotonicNano,
      viewport: nil,
      upsertedNodes: [],
      removedNodeIDs: [],
      gesture: gesture
    )
    if estimatedMemory(afterAdding: frame) > configuration.maximumMemoryBytes {
      await seal(endMonotonicNano: monotonicNano)
      suppressedFrames &+= 1
      return
    }
    frames.append(frame)
    lastMonotonicNano = monotonicNano
    emittedFrames &+= 1
    _ = occurredAtUnixNano
  }

  package func flush() async {
    guard !frames.isEmpty else { return }
    await seal(endMonotonicNano: lastMonotonicNano &+ 1)
  }

  package func resetUnsealed() {
    frames.removeAll(keepingCapacity: false)
    currentNodes.removeAll(keepingCapacity: false)
    currentViewport = nil
    chunkID = nil
    chunkOccurredAtUnixNano = 0
    chunkStartMonotonicNano = 0
    lastMonotonicNano = 0
    currentSessionID = nil
    currentBootID = nil
    replaySessionID = nil
    replayID = nil
    nextChunkIndex = 0
    currentChunkIndex = 0
  }

  package func pendingChunks() -> [ChillReplayChunkDescriptor] {
    guard !consentPurgeRequired else { return [] }
    return store.descriptors()
  }

  package func encryptedData(chunkID: String) throws -> Data {
    guard !consentPurgeRequired else { throw ChillReplayError.chunkNotFound }
    return try store.encryptedData(chunkID: chunkID)
  }

  package func decodedDocument(
    chunkID: String
  ) throws -> ReplayChunkDocument {
    try store.decodedDocument(chunkID: chunkID)
  }

  package func acknowledge(chunkIDs: Set<String>) throws -> Int {
    guard !consentPurgeRequired else { return 0 }
    return try store.acknowledge(chunkIDs: chunkIDs)
  }

  package func purge() throws {
    resetUnsealed()
    consentPurgeRequired = true
    try store.purge()
    consentPurgeRequired = false
  }

  package func metrics() -> ChillReplayMetrics {
    ChillReplayMetrics(
      pendingChunks: store.chunks.count,
      pendingBytes: store.pendingBytes,
      emittedFrames: emittedFrames,
      coalescedFrames: coalescedFrames,
      suppressedFrames: suppressedFrames,
      redactedNodes: redactedNodes,
      evictedChunks: evictedChunks,
      corruptChunks: store.corruptChunks
    )
  }

  package func noteAdmissionCoalesced(_ count: UInt64) {
    coalescedFrames &+= count
  }

  private func startChunk(
    occurredAtUnixNano: UInt64,
    monotonicNano: UInt64,
    sessionID: String,
    bootID: String,
    viewport: ReplayViewport,
    nodes: [ReplayNode]
  ) {
    if replaySessionID != sessionID {
      replaySessionID = sessionID
      replayID = UUIDv7.generate().uuidString.lowercased()
      nextChunkIndex = 0
    }
    chunkID = UUIDv7.generate().uuidString.lowercased()
    currentChunkIndex = nextChunkIndex
    nextChunkIndex &+= 1
    chunkOccurredAtUnixNano = occurredAtUnixNano
    chunkStartMonotonicNano = monotonicNano
    currentSessionID = sessionID
    currentBootID = bootID
    lastMonotonicNano = monotonicNano
    currentViewport = viewport
    currentNodes = Self.indexed(nodes)
    frames = [
      ReplayFrame(
        kind: .snapshot,
        offsetNano: 0,
        viewport: viewport,
        upsertedNodes: nodes,
        removedNodeIDs: [],
        gesture: nil
      )
    ]
    emittedFrames &+= 1
    redactedNodes &+= UInt64(
      nodes.lazy.filter {
        $0.contentMask != nil
      }.count)
  }

  private func shouldRotate(at monotonicNano: UInt64) -> Bool {
    guard !frames.isEmpty else { return false }
    let durationNano = UInt64(configuration.chunkDuration * 1_000_000_000)
    return monotonicNano - chunkStartMonotonicNano >= durationNano
      || frames.count >= configuration.maximumFramesPerChunk
  }

  private func estimatedMemory(afterAdding frame: ReplayFrame) -> Int {
    frames.reduce(0) { $0 + Self.estimatedBytes($1) }
      + Self.estimatedBytes(frame)
  }

  private static func estimatedBytes(_ frame: ReplayFrame) -> Int {
    128 + frame.removedNodeIDs.reduce(0) { $0 + $1.utf8.count + 8 }
      + frame.upsertedNodes.reduce(0) { partial, node in
        partial + 192 + node.id.utf8.count + (node.parentID?.utf8.count ?? 0)
          + (node.semanticID?.utf8.count ?? 0)
      }
  }

  private static func indexed(_ nodes: [ReplayNode]) -> [String: ReplayNode] {
    nodes.reduce(into: [:]) { result, node in result[node.id] = node }
  }

  private func seal(endMonotonicNano: UInt64) async {
    guard let replayID, let chunkID, let sessionID = currentSessionID,
      let bootID = currentBootID,
      !frames.isEmpty
    else { return }
    let end = max(endMonotonicNano, chunkStartMonotonicNano &+ 1)
    var candidateFrames = frames
    while !candidateFrames.isEmpty {
      let document = ReplayChunkDocument(
        schemaVersion: "1.0.0",
        replayID: replayID,
        chunkID: chunkID,
        chunkIndex: currentChunkIndex,
        sessionID: sessionID,
        bootID: bootID,
        occurredAtUnixNano: chunkOccurredAtUnixNano,
        startMonotonicNano: chunkStartMonotonicNano,
        endMonotonicNano: end,
        frames: candidateFrames
      )
      do {
        let result = try store.write(document)
        evictedChunks &+= UInt64(result.evictedCount)
        emitMetadata(result.stored.descriptor)
        break
      } catch ChillReplayError.chunkTooLarge where candidateFrames.count > 1 {
        candidateFrames.removeLast()
        suppressedFrames &+= 1
      } catch {
        suppressedFrames &+= UInt64(candidateFrames.count)
        break
      }
    }
    frames.removeAll(keepingCapacity: true)
    self.chunkID = nil
    currentNodes.removeAll(keepingCapacity: true)
    currentViewport = nil
    chunkOccurredAtUnixNano = 0
    chunkStartMonotonicNano = 0
    lastMonotonicNano = 0
    currentSessionID = nil
    currentBootID = nil
  }

  private func emitMetadata(_ descriptor: ChillReplayChunkDescriptor) {
    guard let runtime = Chill.currentRuntime(),
      runtime.configuration.sessionID.rawValue == descriptor.sessionID,
      runtime.configuration.bootID.rawValue == descriptor.bootID,
      let name = try? SemanticName("session.replay"),
      let subjectID = try? SubjectID(descriptor.replayID),
      let payload = try? ReplayPayload(
        replayID: descriptor.replayID,
        chunkID: descriptor.chunkID,
        chunkIndex: descriptor.chunkIndex,
        startsAtUnixNano: descriptor.occurredAtUnixNano,
        endsAtUnixNano: descriptor.occurredAtUnixNano.addingSaturating(
          descriptor.endMonotonicNano - descriptor.startMonotonicNano
        ),
        sha256: descriptor.digest,
        byteCount: UInt64(descriptor.byteCount),
        storageRef: "replay://chunk/\(descriptor.chunkID)"
      )
    else {
      return
    }
    let redactions =
      frames.first?.upsertedNodes.lazy.filter {
        $0.contentMask != nil
      }.count ?? 0
    _ = try? runtime.record(
      captureClass: .replay,
      at: TimePoint(
        occurredAtUnixNano: descriptor.occurredAtUnixNano,
        monotonicNano: descriptor.startMonotonicNano
      )
    ) {
      try BehaviorDraft(
        subjectID: subjectID,
        kind: .replay,
        operation: .instant,
        name: name,
        redactionState: redactions > 0 ? .applied : .none,
        redactionCount: UInt32(clamping: redactions),
        payload: .replay(payload)
      )
    }
  }
}

extension UInt64 {
  fileprivate func addingSaturating(_ value: UInt64) -> UInt64 {
    let (result, overflow) = addingReportingOverflow(value)
    return overflow ? .max : result
  }
}
