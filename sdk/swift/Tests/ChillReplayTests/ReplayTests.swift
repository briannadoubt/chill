import Foundation
import Testing
import os

@testable import ChillCore
@testable import ChillReplay
@testable import ChillSwiftUI

#if canImport(AppKit)
  import AppKit
#endif

#if canImport(UIKit) && !os(watchOS)
  import UIKit
  #if os(iOS)
    @testable import ChillUIKit
  #endif
#endif

private final class ReplayRecordSink: RecordSink {
  private let storage = OSAllocatedUnfairLock(initialState: [BehaviorRecord]())

  func submit(_ record: BehaviorRecord) {
    storage.withLock { $0.append(record) }
  }

  var records: [BehaviorRecord] { storage.withLock { $0 } }
}

private func temporaryDirectory(_ name: String) throws -> URL {
  let url = FileManager.default.temporaryDirectory.appendingPathComponent(
    "chill-replay-tests-\(name)-\(UUID().uuidString)",
    isDirectory: true
  )
  try FileManager.default.createDirectory(
    at: url,
    withIntermediateDirectories: true
  )
  return url
}

private func replayConfiguration(
  directory: URL,
  keyByte: UInt8 = 7,
  chunkDuration: TimeInterval = 10,
  maximumNodes: Int = 256,
  maximumFrames: Int = 64,
  maximumDiskBytes: Int = 1 * 1_024 * 1_024,
  maximumChunkBytes: Int = 32 * 1_024
) throws -> ChillReplayConfiguration {
  try ChillReplayConfiguration(
    directory: directory,
    encryptionKey: ChillReplayEncryptionKey(
      data: Data(repeating: keyByte, count: 32)
    ),
    captureInterval: 0.25,
    chunkDuration: chunkDuration,
    maximumNodes: maximumNodes,
    maximumFramesPerChunk: maximumFrames,
    maximumMemoryBytes: 1 * 1_024 * 1_024,
    maximumDiskBytes: maximumDiskBytes,
    maximumChunkBytes: maximumChunkBytes
  )
}

private func runtime(
  sink: ReplayRecordSink = ReplayRecordSink()
) throws -> (ChillRuntime, ReplayRecordSink) {
  let runtime = try ChillRuntime(
    configuration: RuntimeConfiguration(
      sessionID: SessionID("session-replay"),
      bootID: BootID("boot-replay"),
      privacy: PrivacyPolicy(
        version: "privacy-replay-v1",
        analytics: .denied,
        replay: .granted
      ),
      sampling: SamplingConfiguration(
        stableKey: "subject-replay",
        salt: "replay-tests-v1",
        behavior: .none,
        replay: .all
      )
    ),
    sink: sink
  )
  return (runtime, sink)
}

private func node(
  id: String = "node-1",
  x: Double = 0,
  mask: ReplayContentMask? = nil,
  semanticID: String? = nil
) -> ReplayNode {
  ReplayNode(
    id: id,
    parentID: nil,
    siblingIndex: 0,
    role: .button,
    frame: ReplayRect(x: x, y: 0, width: 44, height: 44),
    visible: true,
    semanticID: semanticID,
    contentMask: mask
  )
}

private func observation(
  monotonicNano: UInt64,
  nodes: [ReplayNode],
  viewportWidth: Double = 320,
  sessionID: String = "session-replay",
  bootID: String = "boot-replay"
) -> ReplayObservation {
  ReplayObservation(
    sessionID: sessionID,
    bootID: bootID,
    occurredAtUnixNano: 1_700_000_000_000_000_000 + monotonicNano,
    monotonicNano: monotonicNano,
    viewport: ReplayViewport(
      size: ReplaySize(width: viewportWidth, height: 640)
    ),
    nodes: nodes
  )
}

private func document(
  index: Int,
  nodes: [ReplayNode] = [node()]
) -> ReplayChunkDocument {
  let start = UInt64(index * 1_000 + 1)
  return ReplayChunkDocument(
    schemaVersion: "1.0.0",
    replayID: "replay-1",
    chunkID: "chunk-\(index)",
    chunkIndex: UInt32(index),
    sessionID: "session-replay",
    bootID: "boot-replay",
    occurredAtUnixNano: 1_700_000_000_000_000_000 + start,
    startMonotonicNano: start,
    endMonotonicNano: start + 1_000,
    frames: [
      ReplayFrame(
        kind: .snapshot,
        offsetNano: 0,
        viewport: ReplayViewport(
          size: ReplaySize(width: 320, height: 640)
        ),
        upsertedNodes: nodes,
        removedNodeIDs: [],
        gesture: nil
      )
    ]
  )
}

@Suite("Privacy-safe structural replay", .serialized)
struct ReplayTests {
  @Test("Text masks implement the cross-platform length buckets")
  func textLengthBuckets() {
    #expect(ReplayContentMask.text(length: 0).lengthBucket == .empty)
    #expect(ReplayContentMask.text(length: 4).lengthBucket == .tiny)
    #expect(ReplayContentMask.text(length: 16).lengthBucket == .short)
    #expect(ReplayContentMask.text(length: 64).lengthBucket == .medium)
    #expect(ReplayContentMask.text(length: 65).lengthBucket == .long)
    #expect(ReplayContentMask.secureInput.kind == .secureInput)
  }

  @Test("Configuration enforces replay resource and encryption bounds")
  func configurationBounds() throws {
    let directory = try temporaryDirectory("configuration")
    defer { try? FileManager.default.removeItem(at: directory) }
    #expect(throws: ChillReplayError.invalidEncryptionKey) {
      try ChillReplayEncryptionKey(data: Data(repeating: 0, count: 31))
    }
    #expect(throws: ChillReplayError.invalidConfiguration) {
      try ChillReplayConfiguration(
        directory: directory,
        encryptionKey: ChillReplayEncryptionKey(
          data: Data(repeating: 0, count: 32)
        ),
        maximumDiskBytes: 257 * 1_024 * 1_024
      )
    }
  }

  @MainActor
  @Test("Replay has one automatic observer per process")
  func singleConfiguration() async throws {
    let directory = try temporaryDirectory("single-configuration")
    defer { try? FileManager.default.removeItem(at: directory) }
    let configuration = try replayConfiguration(directory: directory)
    try ChillSessionReplay.configure(configuration)
    #expect(throws: ChillReplayError.alreadyConfigured) {
      try ChillSessionReplay.configure(configuration)
    }
    try await ChillSessionReplay.disable(purgePending: true)
  }

  @Test("Keyframes and deltas reconstruct viewport, nodes, and gestures")
  func reconstruction() throws {
    let initial = node(mask: .text(length: 12))
    let changed = node(x: 20, mask: .text(length: 12))
    let gesture = ReplayGesture(
      kind: .pointer,
      phase: .ended,
      location: ReplayPoint(x: 22, y: 12),
      targetNodeID: changed.id
    )
    let frames = [
      ReplayFrame(
        kind: .snapshot,
        offsetNano: 0,
        viewport: ReplayViewport(
          size: ReplaySize(width: 320, height: 640)
        ),
        upsertedNodes: [initial],
        removedNodeIDs: [],
        gesture: nil
      ),
      ReplayFrame(
        kind: .delta,
        offsetNano: 100,
        viewport: ReplayViewport(
          size: ReplaySize(width: 640, height: 320)
        ),
        upsertedNodes: [changed],
        removedNodeIDs: [],
        gesture: nil
      ),
      ReplayFrame(
        kind: .gesture,
        offsetNano: 110,
        viewport: nil,
        upsertedNodes: [],
        removedNodeIDs: [],
        gesture: gesture
      ),
    ]
    let state = try ReplayReconstructor.apply(frames)
    #expect(state.nodes[initial.id] == changed)
    #expect(state.viewport?.size.width == 640)
    #expect(state.gestures == [gesture])
  }

  @Test("Engine emits an aligned encrypted chunk and correlated metadata")
  func engineChunkAndMetadata() async throws {
    let directory = try temporaryDirectory("engine")
    defer {
      Chill.disable()
      try? FileManager.default.removeItem(at: directory)
    }
    let (configuredRuntime, sink) = try runtime()
    Chill.configure(configuredRuntime)
    let engine = try ReplayEngine(
      configuration: replayConfiguration(directory: directory)
    )
    await engine.ingest(
      observation(
        monotonicNano: 1_000,
        nodes: [node(mask: .text(length: 9))]
      )
    )
    await engine.ingest(
      observation(
        monotonicNano: 1_500,
        nodes: [node(x: 12, mask: .text(length: 9))],
        viewportWidth: 640
      )
    )
    await engine.ingest(
      gesture: ReplayGesture(
        kind: .pointer,
        phase: .ended,
        location: ReplayPoint(x: 20, y: 20),
        targetNodeID: "node-1"
      ),
      occurredAtUnixNano: 1_700_000_000_000_001_600,
      monotonicNano: 1_600
    )
    await engine.flush()

    let pending = await engine.pendingChunks()
    #expect(pending.count == 1)
    let descriptor = try #require(pending.first)
    #expect(descriptor.startMonotonicNano == 1_000)
    #expect(descriptor.endMonotonicNano == 1_601)
    let decoded = try await engine.decodedDocument(
      chunkID: descriptor.chunkID
    )
    #expect(decoded.frames.first?.kind == .snapshot)
    #expect(decoded.frames.map(\.kind) == [.snapshot, .delta, .gesture])
    let reconstructed = try ReplayReconstructor.apply(decoded.frames)
    #expect(reconstructed.nodes["node-1"]?.frame.x == 12)
    #expect(reconstructed.gestures.count == 1)
    let metadata = try #require(sink.records.first)
    #expect(metadata.kind == .replay)
    #expect(metadata.clock.bootID.rawValue == descriptor.bootID)
    #expect(metadata.clock.monotonicNano == descriptor.startMonotonicNano)
    guard case .replay(let payload) = metadata.payload else {
      Issue.record("Expected replay metadata")
      return
    }
    #expect(payload.chunkID == descriptor.chunkID)
    #expect(payload.replayID == descriptor.replayID)
    #expect(payload.chunkIndex == descriptor.chunkIndex)
    #expect(payload.sha256 == descriptor.digest)
    #expect(payload.byteCount == UInt64(descriptor.byteCount))
  }

  @Test("Every rotated chunk begins with an independent keyframe")
  func chunkRotation() async throws {
    let directory = try temporaryDirectory("rotation")
    defer { try? FileManager.default.removeItem(at: directory) }
    let engine = try ReplayEngine(
      configuration: replayConfiguration(
        directory: directory,
        chunkDuration: 1
      )
    )
    await engine.ingest(
      observation(monotonicNano: 100, nodes: [node()])
    )
    await engine.ingest(
      observation(monotonicNano: 1_000_000_100, nodes: [node(x: 1)])
    )
    await engine.flush()
    let pending = await engine.pendingChunks()
    #expect(pending.count == 2)
    for chunk in pending {
      let decoded = try await engine.decodedDocument(chunkID: chunk.chunkID)
      #expect(decoded.frames.first?.kind == .snapshot)
      _ = try ReplayReconstructor.apply(decoded.frames)
    }
  }

  @Test("Structural overload respects the hard node admission cap")
  func nodeAdmissionCap() async throws {
    let directory = try temporaryDirectory("node-cap")
    defer { try? FileManager.default.removeItem(at: directory) }
    let engine = try ReplayEngine(
      configuration: replayConfiguration(
        directory: directory,
        maximumNodes: 128
      )
    )
    let nodes = (0..<1_000).map { index in
      node(id: "node-\(index)", x: Double(index))
    }
    await engine.ingest(observation(monotonicNano: 100, nodes: nodes))
    await engine.flush()
    let descriptor = try #require(await engine.pendingChunks().first)
    let decoded = try await engine.decodedDocument(
      chunkID: descriptor.chunkID
    )
    #expect(decoded.frames.first?.upsertedNodes.count == 128)
    _ = try ReplayReconstructor.apply(decoded.frames)
  }

  @Test("Encrypted chunks recover after process death and acknowledge exactly")
  func recoveryAndAcknowledgement() throws {
    let directory = try temporaryDirectory("recovery")
    defer { try? FileManager.default.removeItem(at: directory) }
    let configuration = try replayConfiguration(directory: directory)
    var originalDescriptor: ChillReplayChunkDescriptor?
    do {
      let store = try ReplayChunkStore(configuration: configuration)
      originalDescriptor = try store.write(document(index: 1)).stored.descriptor
      let encrypted = try store.encryptedData(chunkID: "chunk-1")
      #expect(encrypted.starts(with: Data("CHILLRP1\n".utf8)))
      #expect(encrypted.range(of: Data("session-replay".utf8)) == nil)
      #expect(encrypted.range(of: Data("node-1".utf8)) == nil)
    }
    let recovered = try ReplayChunkStore(configuration: configuration)
    #expect(recovered.descriptors() == [originalDescriptor].compactMap { $0 })
    let id = try #require(originalDescriptor?.chunkID)
    #expect(try recovered.acknowledge(chunkIDs: [id]) == 1)
    #expect(recovered.descriptors().isEmpty)
  }

  @Test("Wrong keys and corruption are quarantined without cleartext fallback")
  func wrongKeyQuarantine() throws {
    let directory = try temporaryDirectory("wrong-key")
    defer { try? FileManager.default.removeItem(at: directory) }
    let first = try ReplayChunkStore(
      configuration: replayConfiguration(directory: directory, keyByte: 1)
    )
    _ = try first.write(document(index: 1))
    let second = try ReplayChunkStore(
      configuration: replayConfiguration(directory: directory, keyByte: 2)
    )
    #expect(second.descriptors().isEmpty)
    #expect(second.corruptChunks == 1)
  }

  @Test("Consent withdrawal purges sealed and unsealed replay state")
  func consentWithdrawalPurge() async throws {
    let directory = try temporaryDirectory("revocation")
    defer { try? FileManager.default.removeItem(at: directory) }
    let engine = try ReplayEngine(
      configuration: replayConfiguration(directory: directory)
    )
    let admission = ReplayAdmission(engine: engine)
    admission.submit(observation(monotonicNano: 100, nodes: [node()]))
    await admission.flush()
    #expect(await engine.pendingChunks().count == 1)
    admission.submit(observation(monotonicNano: 200, nodes: [node(x: 1)]))
    await admission.revokeConsent()
    #expect(await engine.pendingChunks().isEmpty)
  }

  @Test("Disk pressure evicts oldest replay chunks and never exceeds the cap")
  func boundedDiskPressure() throws {
    let directory = try temporaryDirectory("pressure")
    defer { try? FileManager.default.removeItem(at: directory) }
    let configuration = try replayConfiguration(directory: directory)
    let store = try ReplayChunkStore(configuration: configuration)
    var evicted = 0
    for index in 0..<240 {
      let nodes = (0..<100).map { nodeIndex in
        node(
          id: "\(index)-\(nodeIndex)-\(UUID().uuidString)",
          x: Double(nodeIndex),
          mask: .text(length: nodeIndex),
          semanticID: "item-\(UUID().uuidString.lowercased())"
        )
      }
      evicted += try store.write(
        document(index: index, nodes: nodes)
      ).evictedCount
    }
    #expect(evicted > 0)
    #expect(store.pendingBytes <= configuration.maximumDiskBytes)
    #expect(store.descriptors().count < 240)
  }

  #if canImport(AppKit)
    @MainActor
    @Test("AppKit masking happens at source and custom drawing is opaque")
    func appKitSourceMasking() throws {
      final class CustomDrawingView: NSView {}

      let secretLabel = "SECRET-CAT-NAME"
      let secretPassword = "SECRET-PASSWORD"
      let root = NSView(frame: NSRect(x: 0, y: 0, width: 400, height: 500))
      let label = NSTextField(labelWithString: secretLabel)
      label.frame = NSRect(x: 10, y: 10, width: 120, height: 20)
      let secure = NSSecureTextField(
        frame: NSRect(x: 10, y: 40, width: 180, height: 24)
      )
      secure.stringValue = secretPassword
      let image = NSImageView(
        frame: NSRect(x: 10, y: 80, width: 80, height: 80)
      )
      let custom = CustomDrawingView(
        frame: NSRect(x: 100, y: 80, width: 80, height: 80)
      )
      let forbiddenChild = NSTextField(labelWithString: "SECRET-CHILD")
      custom.addSubview(forbiddenChild)
      let blockedLabel = NSTextField(labelWithString: "SECRET-BLOCKED")
      blockedLabel.frame = NSRect(x: 220, y: 10, width: 120, height: 20)
      let boundary = ChillReplayPolicyPlatformView(frame: blockedLabel.frame)
      boundary.chillReplayPolicy = ViewCapturePolicy(
        privacy: .blocked,
        replay: .blocked
      )
      root.addSubview(label)
      root.addSubview(secure)
      root.addSubview(image)
      root.addSubview(custom)
      root.addSubview(blockedLabel)
      root.addSubview(boundary)
      let window = NSWindow(
        contentRect: root.bounds,
        styleMask: [.borderless],
        backing: .buffered,
        defer: false
      )
      window.contentView = root
      let (configuredRuntime, _) = try runtime()
      let captured = try #require(
        AppKitReplayTree.snapshot(
          windows: [window],
          runtime: configuredRuntime,
          maximumNodes: 100
        )
      )
      let labelNode = try #require(
        captured.nodes.first {
          $0.id == AppKitReplayTree.nodeID(label)
        })
      let secureNode = try #require(
        captured.nodes.first {
          $0.id == AppKitReplayTree.nodeID(secure)
        })
      let imageNode = try #require(
        captured.nodes.first {
          $0.id == AppKitReplayTree.nodeID(image)
        })
      let customNode = try #require(
        captured.nodes.first {
          $0.id == AppKitReplayTree.nodeID(custom)
        })
      #expect(labelNode.contentMask == .text(length: secretLabel.count))
      #expect(secureNode.contentMask == .secureInput)
      #expect(imageNode.contentMask == .pixels)
      #expect(customNode.contentMask == .customDrawing)
      #expect(captured.nodes.count == 5)
      #expect(
        !captured.nodes.contains {
          $0.id == AppKitReplayTree.nodeID(blockedLabel)
        })

      let encoder = JSONEncoder()
      let frame = ReplayFrame(
        kind: .snapshot,
        offsetNano: 0,
        viewport: captured.viewport,
        upsertedNodes: captured.nodes,
        removedNodeIDs: [],
        gesture: nil
      )
      let bytes = try encoder.encode([frame])
      let serialized = String(decoding: bytes, as: UTF8.self)
      #expect(!serialized.contains(secretLabel))
      #expect(!serialized.contains(secretPassword))
      #expect(!serialized.contains("SECRET-CHILD"))
      #expect(!serialized.contains("SECRET-BLOCKED"))
    }
  #endif

  #if canImport(UIKit) && !os(watchOS)
    @MainActor
    @Test("UIKit and SwiftUI policies mask content before structural admission")
    func uiKitSourceMasking() throws {
      final class CustomDrawingView: UIView {}

      let secretLabel = "SECRET-IOS-LABEL"
      let secretPassword = "SECRET-IOS-PASSWORD"
      let root = UIView(frame: CGRect(x: 0, y: 0, width: 400, height: 500))
      let label = UILabel(frame: CGRect(x: 10, y: 10, width: 120, height: 20))
      label.text = secretLabel
      let secure = UITextField(
        frame: CGRect(x: 10, y: 40, width: 180, height: 24)
      )
      secure.isSecureTextEntry = true
      secure.text = secretPassword
      let image = UIImageView(
        frame: CGRect(x: 10, y: 80, width: 80, height: 80)
      )
      let custom = CustomDrawingView(
        frame: CGRect(x: 100, y: 80, width: 80, height: 80)
      )
      let forbiddenChild = UILabel(frame: custom.bounds)
      forbiddenChild.text = "SECRET-IOS-CHILD"
      custom.addSubview(forbiddenChild)
      let blockedLabel = UILabel(
        frame: CGRect(x: 220, y: 10, width: 120, height: 20)
      )
      blockedLabel.text = "SECRET-IOS-BLOCKED"
      let boundary = ChillReplayPolicyPlatformView(frame: blockedLabel.frame)
      boundary.chillReplayPolicy = ViewCapturePolicy(
        privacy: .blocked,
        replay: .blocked
      )
      root.addSubview(label)
      root.addSubview(secure)
      root.addSubview(image)
      root.addSubview(custom)
      root.addSubview(blockedLabel)
      root.addSubview(boundary)
      #if os(iOS)
        let declaredBlockedContainer = UIView(
          frame: CGRect(x: 220, y: 80, width: 120, height: 40)
        ).replay(.blocked)
        let declaredBlockedLabel = UILabel(frame: declaredBlockedContainer.bounds)
        declaredBlockedLabel.text = "SECRET-IOS-DECLARED-BLOCK"
        declaredBlockedContainer.addSubview(declaredBlockedLabel)
        root.addSubview(declaredBlockedContainer)
      #endif
      let controller = UIViewController()
      controller.view = root
      let window = UIWindow(frame: root.bounds)
      window.rootViewController = controller
      window.isHidden = false
      let (configuredRuntime, _) = try runtime()
      let captured = try #require(
        UIKitReplayTree.snapshot(
          windows: [window],
          runtime: configuredRuntime,
          maximumNodes: 100
        )
      )
      let labelNode = try #require(
        captured.nodes.first {
          $0.id == UIKitReplayTree.nodeID(label)
        })
      let secureNode = try #require(
        captured.nodes.first {
          $0.id == UIKitReplayTree.nodeID(secure)
        })
      let imageNode = try #require(
        captured.nodes.first {
          $0.id == UIKitReplayTree.nodeID(image)
        })
      let customNode = try #require(
        captured.nodes.first {
          $0.id == UIKitReplayTree.nodeID(custom)
        })
      #expect(labelNode.contentMask == .text(length: secretLabel.count))
      #expect(secureNode.contentMask == .secureInput)
      #expect(imageNode.contentMask == .pixels)
      #expect(customNode.contentMask == .customDrawing)
      #expect(
        !captured.nodes.contains {
          $0.id == UIKitReplayTree.nodeID(blockedLabel)
        })
      #expect(
        !captured.nodes.contains {
          $0.id == UIKitReplayTree.nodeID(forbiddenChild)
        })
      #if os(iOS)
        #expect(
          !captured.nodes.contains {
            $0.id == UIKitReplayTree.nodeID(declaredBlockedContainer)
          })
        #expect(
          !captured.nodes.contains {
            $0.id == UIKitReplayTree.nodeID(declaredBlockedLabel)
          })
      #endif

      let frame = ReplayFrame(
        kind: .snapshot,
        offsetNano: 0,
        viewport: captured.viewport,
        upsertedNodes: captured.nodes,
        removedNodeIDs: [],
        gesture: nil
      )
      let bytes = try JSONEncoder().encode([frame])
      let serialized = String(decoding: bytes, as: UTF8.self)
      #expect(!serialized.contains(secretLabel))
      #expect(!serialized.contains(secretPassword))
      #expect(!serialized.contains("SECRET-IOS-CHILD"))
      #expect(!serialized.contains("SECRET-IOS-BLOCKED"))
      #if os(iOS)
        #expect(!serialized.contains("SECRET-IOS-DECLARED-BLOCK"))
      #endif
    }
  #endif
}
