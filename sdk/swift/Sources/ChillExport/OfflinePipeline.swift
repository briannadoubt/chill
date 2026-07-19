import ChillCore
import Foundation
import os

/// The durable sink used by `ChillRuntime`. `submit` performs only bounded
/// in-memory admission; encoding, files, compression, and networking run on a
/// utility task behind the sink.
public final class ChillOfflinePipeline: RecordSink, Sendable {
  private struct PendingRecord: Sendable {
    let record: BehaviorRecord
    let priority: ChillQueuePriority
  }

  private struct State {
    var pending: [PendingRecord] = []
    var drainScheduled = false
    var accepting = true
    var waiters: [CheckedContinuation<Void, Never>] = []
    var droppedReplay: UInt64 = 0
    var droppedLow: UInt64 = 0
    var droppedHigh: UInt64 = 0
  }

  private let configuration: ChillOTLPConfiguration
  private let state = OSAllocatedUnfairLock(initialState: State())
  private let worker: OfflineExportWorker

  public convenience init(
    directory: URL,
    configuration: ChillOTLPConfiguration
  ) throws {
    let queue = try DurableRecordQueue(
      directory: directory,
      maximumBytes: configuration.offline.maximumDiskBytes
    )
    let transport = OTLPHTTPTransport(configuration: configuration)
    let sourceMetadata = try ChillSourceMetadata.persistent(in: directory)
    self.init(
      configuration: configuration,
      queue: queue,
      transport: transport,
      sourceMetadata: sourceMetadata
    )
  }

  package init(
    configuration: ChillOTLPConfiguration,
    queue: DurableRecordQueue,
    transport: any OTLPExportTransport,
    sourceMetadata: ChillSourceMetadata? = nil
  ) {
    self.configuration = configuration
    worker = OfflineExportWorker(
      queue: queue,
      configuration: configuration,
      transport: transport,
      sourceMetadata: sourceMetadata
    )
    Task { await worker.start() }
  }

  @inline(__always)
  public func submit(_ record: BehaviorRecord) {
    let incoming = PendingRecord(
      record: record,
      priority: Self.priority(for: record)
    )
    let shouldSchedule = state.withLock { state -> Bool in
      guard state.accepting else {
        Self.incrementDrop(incoming.priority, state: &state)
        return false
      }
      if state.pending.count >= configuration.offline.maximumMemoryRecords {
        if let victim = Self.memoryEvictionCandidate(
          for: incoming.priority,
          in: state.pending
        ) {
          let evicted = state.pending.remove(at: victim)
          Self.incrementDrop(evicted.priority, state: &state)
        } else {
          Self.incrementDrop(incoming.priority, state: &state)
          return false
        }
      }
      state.pending.append(incoming)
      guard !state.drainScheduled else { return false }
      state.drainScheduled = true
      return true
    }
    if shouldSchedule {
      Task.detached(priority: .utility) { [weak self] in
        await self?.drainPendingRecords()
      }
    }
  }

  /// Persists all records admitted before this call and attempts delivery until
  /// the queue is empty or one retryable/blocked response is observed.
  public func flush() async -> ChillFlushResult {
    await waitForCurrentDrain()
    return await worker.flush()
  }

  /// Stops admission, persists the admitted prefix, and performs one complete
  /// flush cycle. Durable unacknowledged records remain for the next launch.
  public func shutdown() async -> ChillFlushResult {
    state.withLock { $0.accepting = false }
    await waitForCurrentDrain()
    let result = await worker.flush()
    await worker.shutdown()
    return result
  }

  public func metrics() async -> ChillExporterMetrics {
    let memory = state.withLock { state in
      MemoryMetrics(
        pending: state.pending.count,
        droppedReplay: state.droppedReplay,
        droppedLow: state.droppedLow,
        droppedHigh: state.droppedHigh
      )
    }
    return await worker.metrics(memory: memory)
  }

  private func drainPendingRecords() async {
    while true {
      let result = state.withLock { state -> DrainResult in
        guard !state.pending.isEmpty else {
          state.drainScheduled = false
          let waiters = state.waiters
          state.waiters.removeAll(keepingCapacity: true)
          return .finished(waiters)
        }
        let records = state.pending
        state.pending.removeAll(keepingCapacity: true)
        return .records(records)
      }
      switch result {
      case .records(let records):
        await worker.persist(records.map(\.record))
      case .finished(let waiters):
        for waiter in waiters { waiter.resume() }
        return
      }
    }
  }

  private func waitForCurrentDrain() async {
    await withCheckedContinuation { continuation in
      let isComplete = state.withLock { state -> Bool in
        guard state.drainScheduled || !state.pending.isEmpty else {
          return true
        }
        state.waiters.append(continuation)
        return false
      }
      if isComplete { continuation.resume() }
    }
  }

  private static func priority(for record: BehaviorRecord) -> ChillQueuePriority {
    switch record.kind {
    case .replay: .replay
    case .impression: .low
    default: .high
    }
  }

  private static func memoryEvictionCandidate(
    for incoming: ChillQueuePriority,
    in records: [PendingRecord]
  ) -> Int? {
    records.indices
      .filter { index in
        let priority = records[index].priority
        if priority.rawValue < incoming.rawValue { return true }
        return priority == incoming && incoming != .high
      }
      .min { left, right in
        records[left].priority.rawValue < records[right].priority.rawValue
      }
  }

  private static func incrementDrop(
    _ priority: ChillQueuePriority,
    state: inout State
  ) {
    switch priority {
    case .replay: state.droppedReplay &+= 1
    case .low: state.droppedLow &+= 1
    case .high: state.droppedHigh &+= 1
    }
  }

  private enum DrainResult {
    case records([PendingRecord])
    case finished([CheckedContinuation<Void, Never>])
  }
}

private struct MemoryMetrics: Sendable {
  let pending: Int
  let droppedReplay: UInt64
  let droppedLow: UInt64
  let droppedHigh: UInt64
}

private actor OfflineExportWorker {
  private struct MetricsState {
    var persisted: UInt64 = 0
    var acknowledged: UInt64 = 0
    var duplicates: UInt64 = 0
    var droppedReplay: UInt64 = 0
    var droppedLow: UInt64 = 0
    var droppedHigh: UInt64 = 0
    var exportAttempts: UInt64 = 0
    var exportFailures: UInt64 = 0
    var consecutiveFailures: UInt32 = 0
  }

  private enum ExportStep {
    case empty
    case progressed
    case retry
    case blocked
    case busy
  }

  private let queue: DurableRecordQueue
  private let configuration: ChillOTLPConfiguration
  private let transport: any OTLPExportTransport
  private let sourceMetadata: ChillSourceMetadata?
  private var metricsState = MetricsState()
  private var flushTask: Task<Void, Never>?
  private var retryTask: Task<Void, Never>?
  private var diskRetryTask: Task<Void, Never>?
  private var volatileRecords: [BehaviorRecord] = []
  private var exporting = false
  private var retryAttempt = 0
  private var stopped = false

  init(
    queue: DurableRecordQueue,
    configuration: ChillOTLPConfiguration,
    transport: any OTLPExportTransport,
    sourceMetadata: ChillSourceMetadata?
  ) {
    self.queue = queue
    self.configuration = configuration
    self.transport = transport
    self.sourceMetadata = sourceMetadata
  }

  func start() {
    if queue.count > 0 { scheduleFlush() }
  }

  func persist(_ records: [BehaviorRecord]) {
    guard !stopped else { return }
    let candidates = volatileRecords + records
    volatileRecords.removeAll(keepingCapacity: true)
    for record in candidates {
      let priority = Self.priority(for: record)
      do {
        let payload = OTLPProtobufEncoder.encodeLogRecord(
          record,
          sourceMetadata: sourceMetadata
        )
        let result = try queue.append(
          recordID: record.recordID.rawValue,
          kind: record.kind.rawValue,
          priority: priority,
          occurredAtUnixNano: record.clock.occurredAtUnixNano,
          sequenceNumber: record.clock.sequenceNumber,
          payload: payload
        )
        if result.persisted { metricsState.persisted &+= 1 }
        if result.duplicate { metricsState.duplicates &+= 1 }
        for dropped in result.dropped { recordDrop(dropped.priority) }
      } catch {
        admitVolatile(record)
      }
    }
    if !volatileRecords.isEmpty { schedulePersistenceRetry() }
    guard queue.count > 0 else { return }
    if queue.count >= configuration.offline.maximumBatchRecords {
      Task { [weak self] in await self?.automaticExport() }
    } else {
      scheduleFlush()
    }
  }

  func flush() async -> ChillFlushResult {
    flushTask?.cancel()
    flushTask = nil
    retryTask?.cancel()
    retryTask = nil
    diskRetryTask?.cancel()
    diskRetryTask = nil
    if !volatileRecords.isEmpty {
      persist([])
      if !volatileRecords.isEmpty {
        schedulePersistenceRetry()
        return .retryScheduled
      }
    }
    var remainingSteps = max(1, queue.count + 1)
    while remainingSteps > 0 {
      switch await exportOne() {
      case .empty:
        return .emptied
      case .progressed:
        remainingSteps -= 1
        continue
      case .retry:
        return .retryScheduled
      case .blocked:
        return .blocked
      case .busy:
        try? await Task.sleep(for: .milliseconds(1))
        continue
      }
    }
    return queue.count == 0 ? .emptied : .retryScheduled
  }

  func shutdown() {
    stopped = true
    flushTask?.cancel()
    retryTask?.cancel()
    diskRetryTask?.cancel()
    flushTask = nil
    retryTask = nil
    diskRetryTask = nil
  }

  func metrics(memory: MemoryMetrics) -> ChillExporterMetrics {
    ChillExporterMetrics(
      queuedRecords: queue.count,
      queuedBytes: queue.queuedBytes,
      memoryPendingRecords: memory.pending + volatileRecords.count,
      recoveredRecords: queue.recoveredRecords,
      quarantinedFiles: queue.quarantinedFiles,
      persistedRecords: metricsState.persisted,
      acknowledgedRecords: metricsState.acknowledged,
      duplicateRecords: metricsState.duplicates,
      droppedReplayRecords: metricsState.droppedReplay + memory.droppedReplay,
      droppedLowPriorityRecords: metricsState.droppedLow + memory.droppedLow,
      droppedHighPriorityRecords: metricsState.droppedHigh + memory.droppedHigh,
      exportAttempts: metricsState.exportAttempts,
      exportFailures: metricsState.exportFailures,
      consecutiveFailures: metricsState.consecutiveFailures
    )
  }

  private func automaticExport() async {
    guard !stopped else { return }
    flushTask?.cancel()
    flushTask = nil
    switch await exportOne() {
    case .progressed where queue.count > 0:
      if queue.count >= configuration.offline.maximumBatchRecords {
        Task { [weak self] in await self?.automaticExport() }
      } else {
        scheduleFlush()
      }
    case .busy:
      scheduleFlush()
    case .empty, .progressed, .retry, .blocked:
      break
    }
  }

  private func exportOne() async -> ExportStep {
    guard !stopped else { return .blocked }
    guard !exporting else { return .busy }
    let entries = queue.peek(
      maximumRecords: configuration.offline.maximumBatchRecords,
      maximumPayloadBytes: configuration.offline.maximumBatchBytes
    )
    guard !entries.isEmpty else { return .empty }
    let body = OTLPProtobufEncoder.encodeBatch(
      entries,
      resourceAttributes: configuration.resourceAttributes
    )
    exporting = true
    metricsState.exportAttempts &+= 1
    let recordIDs = Set(entries.map(\.recordID))
    let disposition = await transport.export(
      OTLPExportRequest(recordIDs: recordIDs, body: body)
    )
    exporting = false
    switch disposition {
    case .acknowledged(let acknowledged):
      let accepted = acknowledged.intersection(recordIDs)
      guard !accepted.isEmpty else {
        recordFailure()
        scheduleRetry(serverDelay: nil)
        return .retry
      }
      do {
        let count = try queue.acknowledge(recordIDs: accepted)
        metricsState.acknowledged &+= UInt64(count)
        metricsState.consecutiveFailures = 0
        retryAttempt = 0
        return .progressed
      } catch {
        recordFailure()
        scheduleRetry(serverDelay: nil)
        return .retry
      }
    case .retry(let delay):
      recordFailure()
      scheduleRetry(serverDelay: delay)
      return .retry
    case .blocked:
      recordFailure()
      return .blocked
    }
  }

  private func scheduleFlush() {
    guard flushTask == nil, queue.count > 0, !stopped else { return }
    let interval = configuration.offline.flushInterval
    flushTask = Task { [weak self] in
      try? await Task.sleep(for: .seconds(interval))
      guard !Task.isCancelled else { return }
      await self?.flushTimerFired()
    }
  }

  private func flushTimerFired() async {
    flushTask = nil
    await automaticExport()
  }

  private func scheduleRetry(serverDelay: TimeInterval?) {
    guard retryTask == nil, queue.count > 0, !stopped else { return }
    flushTask?.cancel()
    flushTask = nil
    let computed = configuration.offline.retry.delay(
      attempt: retryAttempt,
      unitRandom: Double.random(in: 0...1)
    )
    retryAttempt = min(retryAttempt + 1, 30)
    let delay = min(3_600, max(0.01, serverDelay ?? computed))
    retryTask = Task { [weak self] in
      try? await Task.sleep(for: .seconds(delay))
      guard !Task.isCancelled else { return }
      await self?.retryTimerFired()
    }
  }

  private func retryTimerFired() async {
    retryTask = nil
    await automaticExport()
  }

  private func schedulePersistenceRetry() {
    guard diskRetryTask == nil, !volatileRecords.isEmpty, !stopped else {
      return
    }
    let delay = configuration.offline.retry.initialDelay
    diskRetryTask = Task { [weak self] in
      try? await Task.sleep(for: .seconds(delay))
      guard !Task.isCancelled else { return }
      await self?.persistenceRetryTimerFired()
    }
  }

  private func persistenceRetryTimerFired() {
    diskRetryTask = nil
    persist([])
  }

  private func admitVolatile(_ record: BehaviorRecord) {
    let incoming = Self.priority(for: record)
    let limit = configuration.offline.maximumMemoryRecords
    if volatileRecords.count < limit {
      volatileRecords.append(record)
      return
    }
    let victim = volatileRecords.indices
      .filter { index in
        let priority = Self.priority(for: volatileRecords[index])
        if priority.rawValue < incoming.rawValue { return true }
        return priority == incoming && incoming != .high
      }
      .min { left, right in
        Self.priority(for: volatileRecords[left]).rawValue
          < Self.priority(for: volatileRecords[right]).rawValue
      }
    guard let victim else {
      recordDrop(incoming)
      return
    }
    recordDrop(Self.priority(for: volatileRecords[victim]))
    volatileRecords.remove(at: victim)
    volatileRecords.append(record)
  }

  private func recordFailure() {
    metricsState.exportFailures &+= 1
    metricsState.consecutiveFailures &+= 1
  }

  private func recordDrop(_ priority: ChillQueuePriority) {
    switch priority {
    case .replay: metricsState.droppedReplay &+= 1
    case .low: metricsState.droppedLow &+= 1
    case .high: metricsState.droppedHigh &+= 1
    }
  }

  private static func priority(for record: BehaviorRecord) -> ChillQueuePriority {
    switch record.kind {
    case .replay: .replay
    case .impression: .low
    default: .high
    }
  }
}
