import Foundation

@MainActor
package protocol ReplayPlatformObserving: AnyObject {
  func start()
  func stop()
}

@MainActor
private final class ReplayController {
  let engine: ReplayEngine
  let admission: ReplayAdmission
  let observer: any ReplayPlatformObserving

  init(
    engine: ReplayEngine,
    admission: ReplayAdmission,
    observer: any ReplayPlatformObserving
  ) {
    self.engine = engine
    self.admission = admission
    self.observer = observer
  }
}

/// Automatic, structural Apple session replay. Configuration is the only app
/// integration point; UI facts, gestures, scrolling, chunking, and lifecycle
/// flushes are observed by the native adapters.
@MainActor
public enum ChillSessionReplay {
  private static var controller: ReplayController?

  public static func configure(
    _ configuration: ChillReplayConfiguration
  ) throws {
    guard controller == nil else { throw ChillReplayError.alreadyConfigured }
    let engine = try ReplayEngine(configuration: configuration)
    let admission = ReplayAdmission(engine: engine)
    let observer = makeReplayPlatformObserver(
      admission: admission,
      interval: configuration.captureInterval,
      maximumNodes: configuration.maximumNodes
    )
    let configured = ReplayController(
      engine: engine,
      admission: admission,
      observer: observer
    )
    controller = configured
    configured.observer.start()
  }

  /// Stops automatic observation. The unsealed in-memory prefix is discarded
  /// immediately; already sealed chunks remain unless `purgePending` is true.
  public static func disable(purgePending: Bool = false) async throws {
    guard let configured = controller else { return }
    configured.observer.stop()
    await configured.admission.discardUnsealed()
    if purgePending { try await configured.engine.purge() }
    controller = nil
  }

  /// Seals the current structural chunk. Native background notifications call
  /// this automatically; it is public for deterministic lifecycle integration.
  public static func flush() async {
    await controller?.admission.flush()
  }

  public static func pendingChunks() async -> [ChillReplayChunkDescriptor] {
    guard let engine = controller?.engine else { return [] }
    return await engine.pendingChunks()
  }

  /// Returns the encrypted replay envelope for the service uploader. Cleartext
  /// frames are never exposed by the public SDK.
  public static func encryptedPayload(for chunkID: String) async throws -> Data {
    guard let engine = controller?.engine else {
      throw ChillReplayError.chunkNotFound
    }
    return try await engine.encryptedData(chunkID: chunkID)
  }

  @discardableResult
  public static func acknowledge(
    chunkIDs: Set<String>
  ) async throws -> Int {
    guard let engine = controller?.engine else { return 0 }
    return try await engine.acknowledge(chunkIDs: chunkIDs)
  }

  public static func metrics() async -> ChillReplayMetrics {
    guard let engine = controller?.engine else {
      return ChillReplayMetrics(
        pendingChunks: 0,
        pendingBytes: 0,
        emittedFrames: 0,
        coalescedFrames: 0,
        suppressedFrames: 0,
        redactedNodes: 0,
        evictedChunks: 0,
        corruptChunks: 0
      )
    }
    return await engine.metrics()
  }
}

@MainActor
private func makeReplayPlatformObserver(
  admission: ReplayAdmission,
  interval: TimeInterval,
  maximumNodes: Int
) -> any ReplayPlatformObserving {
  #if canImport(UIKit) && !os(watchOS)
    UIKitReplayObserver(
      admission: admission,
      interval: interval,
      maximumNodes: maximumNodes
    )
  #elseif canImport(AppKit)
    AppKitReplayObserver(
      admission: admission,
      interval: interval,
      maximumNodes: maximumNodes
    )
  #else
    NoOpReplayObserver()
  #endif
}

@MainActor
private final class NoOpReplayObserver: ReplayPlatformObserving {
  func start() {}
  func stop() {}
}
