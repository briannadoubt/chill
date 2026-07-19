import Foundation

package struct ReplayPoint: Codable, Equatable, Sendable {
  package let x: Double
  package let y: Double

  package init(x: Double, y: Double) {
    self.x = x.rounded(toPlaces: 2)
    self.y = y.rounded(toPlaces: 2)
  }
}

package struct ReplaySize: Codable, Equatable, Sendable {
  package let width: Double
  package let height: Double

  package init(width: Double, height: Double) {
    self.width = max(0, width.rounded(toPlaces: 2))
    self.height = max(0, height.rounded(toPlaces: 2))
  }
}

package struct ReplayRect: Codable, Equatable, Sendable {
  package let x: Double
  package let y: Double
  package let width: Double
  package let height: Double

  package init(x: Double, y: Double, width: Double, height: Double) {
    self.x = x.rounded(toPlaces: 2)
    self.y = y.rounded(toPlaces: 2)
    self.width = max(0, width.rounded(toPlaces: 2))
    self.height = max(0, height.rounded(toPlaces: 2))
  }
}

package enum ReplayRole: String, Codable, Sendable {
  case viewport
  case container
  case text
  case textInput = "text_input"
  case button
  case toggle
  case slider
  case picker
  case image
  case scroll
  case web
  case media
  case custom
}

package enum ReplayContentMaskKind: String, Codable, Sendable {
  case text
  case secureInput = "secure_input"
  case pixels
  case customDrawing = "custom_drawing"
}

package enum ReplayTextLengthBucket: String, Codable, Sendable {
  case empty = "0"
  case tiny = "1-4"
  case short = "5-16"
  case medium = "17-64"
  case long = "65+"

  package init(length: Int) {
    switch length {
    case ...0: self = .empty
    case 1...4: self = .tiny
    case 5...16: self = .short
    case 17...64: self = .medium
    default: self = .long
    }
  }
}

package struct ReplayContentMask: Codable, Equatable, Sendable {
  package let kind: ReplayContentMaskKind
  package let lengthBucket: ReplayTextLengthBucket?

  package static func text(length: Int) -> ReplayContentMask {
    ReplayContentMask(
      kind: .text,
      lengthBucket: ReplayTextLengthBucket(length: length)
    )
  }

  package static let secureInput = ReplayContentMask(
    kind: .secureInput,
    lengthBucket: nil
  )
  package static let pixels = ReplayContentMask(
    kind: .pixels,
    lengthBucket: nil
  )
  package static let customDrawing = ReplayContentMask(
    kind: .customDrawing,
    lengthBucket: nil
  )
}

package struct ReplayScrollState: Codable, Equatable, Sendable {
  package let offset: ReplayPoint
  package let contentSize: ReplaySize
}

package struct ReplayNode: Codable, Equatable, Sendable {
  package let id: String
  package let parentID: String?
  package let siblingIndex: Int
  package let role: ReplayRole
  package let frame: ReplayRect
  package let visible: Bool
  package let enabled: Bool
  package let selected: Bool
  package let focused: Bool
  package let opacity: Double
  package let semanticID: String?
  package let contentMask: ReplayContentMask?
  package let scroll: ReplayScrollState?

  package init(
    id: String,
    parentID: String?,
    siblingIndex: Int,
    role: ReplayRole,
    frame: ReplayRect,
    visible: Bool,
    enabled: Bool = true,
    selected: Bool = false,
    focused: Bool = false,
    opacity: Double = 1,
    semanticID: String? = nil,
    contentMask: ReplayContentMask? = nil,
    scroll: ReplayScrollState? = nil
  ) {
    self.id = id
    self.parentID = parentID
    self.siblingIndex = max(0, siblingIndex)
    self.role = role
    self.frame = frame
    self.visible = visible
    self.enabled = enabled
    self.selected = selected
    self.focused = focused
    self.opacity = min(1, max(0, opacity.rounded(toPlaces: 3)))
    self.semanticID = semanticID
    self.contentMask = contentMask
    self.scroll = scroll
  }
}

package struct ReplayViewport: Codable, Equatable, Sendable {
  package let size: ReplaySize
  package let safeAreaTop: Double
  package let safeAreaLeft: Double
  package let safeAreaBottom: Double
  package let safeAreaRight: Double

  package init(
    size: ReplaySize,
    safeAreaTop: Double = 0,
    safeAreaLeft: Double = 0,
    safeAreaBottom: Double = 0,
    safeAreaRight: Double = 0
  ) {
    self.size = size
    self.safeAreaTop = max(0, safeAreaTop.rounded(toPlaces: 2))
    self.safeAreaLeft = max(0, safeAreaLeft.rounded(toPlaces: 2))
    self.safeAreaBottom = max(0, safeAreaBottom.rounded(toPlaces: 2))
    self.safeAreaRight = max(0, safeAreaRight.rounded(toPlaces: 2))
  }
}

package struct ReplayObservation: Equatable, Sendable {
  package let sessionID: String
  package let bootID: String
  package let occurredAtUnixNano: UInt64
  package let monotonicNano: UInt64
  package let viewport: ReplayViewport
  package let nodes: [ReplayNode]
}

package enum ReplayGestureKind: String, Codable, Sendable {
  case pointer
  case drag
  case scroll
}

package enum ReplayGesturePhase: String, Codable, Sendable {
  case began
  case changed
  case ended
  case cancelled
}

package struct ReplayGesture: Codable, Equatable, Sendable {
  package let kind: ReplayGestureKind
  package let phase: ReplayGesturePhase
  package let location: ReplayPoint
  package let targetNodeID: String?
}

package enum ReplayFrameKind: String, Codable, Sendable {
  case snapshot
  case delta
  case gesture
}

package struct ReplayFrame: Codable, Equatable, Sendable {
  package let kind: ReplayFrameKind
  package let offsetNano: UInt64
  package let viewport: ReplayViewport?
  package let upsertedNodes: [ReplayNode]
  package let removedNodeIDs: [String]
  package let gesture: ReplayGesture?
}

package struct ReplayChunkDocument: Codable, Equatable, Sendable {
  package let schemaVersion: String
  package let replayID: String
  package let chunkID: String
  package let chunkIndex: UInt32
  package let sessionID: String
  package let bootID: String
  package let occurredAtUnixNano: UInt64
  package let startMonotonicNano: UInt64
  package let endMonotonicNano: UInt64
  package let frames: [ReplayFrame]
}

package struct ReconstructedReplayState: Equatable, Sendable {
  package let viewport: ReplayViewport?
  package let nodes: [String: ReplayNode]
  package let gestures: [ReplayGesture]
}

package enum ReplayReconstructor {
  package static func apply(
    _ frames: [ReplayFrame]
  ) throws -> ReconstructedReplayState {
    guard frames.first?.kind == .snapshot else {
      throw ChillReplayError.corruptChunk
    }
    var nodes: [String: ReplayNode] = [:]
    var viewport: ReplayViewport?
    var gestures: [ReplayGesture] = []
    for frame in frames {
      if let updatedViewport = frame.viewport { viewport = updatedViewport }
      if frame.kind == .snapshot { nodes.removeAll(keepingCapacity: true) }
      for id in frame.removedNodeIDs { nodes.removeValue(forKey: id) }
      for node in frame.upsertedNodes { nodes[node.id] = node }
      if let gesture = frame.gesture { gestures.append(gesture) }
    }
    return ReconstructedReplayState(
      viewport: viewport,
      nodes: nodes,
      gestures: gestures
    )
  }
}

extension Double {
  fileprivate func rounded(toPlaces places: Int) -> Double {
    guard isFinite else { return 0 }
    let factor = pow(10, Double(places))
    return (self * factor).rounded() / factor
  }
}
