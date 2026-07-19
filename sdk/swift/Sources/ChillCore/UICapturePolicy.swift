public enum ViewPrivacyDisposition: Int, Codable, Sendable {
  /// Ordinary semantic facts are eligible subject to runtime consent.
  case standard
  /// Facts remain eligible, but source-derived content must be redacted.
  case redacted
  /// No facts may be produced from this UI subtree.
  case blocked
}

public enum ReplayDisposition: Int, Codable, Sendable {
  /// Structural replay is eligible subject to runtime consent and policy.
  case automatic
  /// Replay geometry is eligible, but content must be represented by masks.
  case masked
  /// No replay facts may be produced from this UI subtree.
  case blocked
}

public struct ViewCapturePolicy: Equatable, Sendable {
  public let privacy: ViewPrivacyDisposition
  public let replay: ReplayDisposition

  public static let standard = ViewCapturePolicy(
    privacy: .standard,
    replay: .automatic
  )

  public init(
    privacy: ViewPrivacyDisposition = .standard,
    replay: ReplayDisposition = .automatic
  ) {
    self.privacy = privacy
    self.replay = replay
  }

  public func restricting(
    privacy: ViewPrivacyDisposition? = nil,
    replay: ReplayDisposition? = nil
  ) -> ViewCapturePolicy {
    ViewCapturePolicy(
      privacy: max(self.privacy, privacy ?? self.privacy),
      replay: max(self.replay, replay ?? self.replay)
    )
  }
}

extension ViewPrivacyDisposition: Comparable {
  public static func < (
    left: ViewPrivacyDisposition,
    right: ViewPrivacyDisposition
  ) -> Bool {
    left.rawValue < right.rawValue
  }
}

extension ReplayDisposition: Comparable {
  public static func < (
    left: ReplayDisposition,
    right: ReplayDisposition
  ) -> Bool {
    left.rawValue < right.rawValue
  }
}

extension ViewCapturePolicy {
  package var redactionState: RedactionState {
    privacy == .redacted ? .applied : .none
  }
}
