import ChillCore
import SwiftUI

#if canImport(UIKit) && !os(watchOS)
  import UIKit

  @MainActor
  package final class ChillReplayPolicyPlatformView: UIView,
    ReplayPolicyBoundary
  {
    package var chillReplayPolicy = ViewCapturePolicy.standard

    package override init(frame: CGRect) {
      super.init(frame: frame)
      backgroundColor = .clear
      isOpaque = false
      isUserInteractionEnabled = false
      isAccessibilityElement = false
      accessibilityElementsHidden = true
    }

    @available(*, unavailable)
    package required init?(coder _: NSCoder) { nil }
  }

  private struct ChillReplayPolicyProbe: UIViewRepresentable {
    let policy: ViewCapturePolicy

    func makeUIView(context _: Context) -> ChillReplayPolicyPlatformView {
      ChillReplayPolicyPlatformView(frame: .zero)
    }

    func updateUIView(
      _ view: ChillReplayPolicyPlatformView,
      context _: Context
    ) {
      view.chillReplayPolicy = policy
    }
  }
#elseif canImport(AppKit)
  import AppKit

  @MainActor
  package final class ChillReplayPolicyPlatformView: NSView,
    ReplayPolicyBoundary
  {
    package var chillReplayPolicy = ViewCapturePolicy.standard

    package override init(frame frameRect: NSRect) {
      super.init(frame: frameRect)
      setAccessibilityElement(false)
    }

    @available(*, unavailable)
    package required init?(coder _: NSCoder) { nil }

    package override func hitTest(_: NSPoint) -> NSView? { nil }
  }

  private struct ChillReplayPolicyProbe: NSViewRepresentable {
    let policy: ViewCapturePolicy

    func makeNSView(context _: Context) -> ChillReplayPolicyPlatformView {
      ChillReplayPolicyPlatformView(frame: .zero)
    }

    func updateNSView(
      _ view: ChillReplayPolicyPlatformView,
      context _: Context
    ) {
      view.chillReplayPolicy = policy
    }
  }
#else
  private struct ChillReplayPolicyProbe: View {
    let policy: ViewCapturePolicy
    var body: some View { EmptyView() }
  }
#endif

private struct ChillCapturePolicyModifier: ViewModifier {
  @Environment(\.chillCapturePolicy) private var inherited

  let privacy: ViewPrivacyDisposition?
  let replay: ReplayDisposition?

  func body(content: Content) -> some View {
    let effective = inherited.restricting(privacy: privacy, replay: replay)
    content
      .environment(\.chillCapturePolicy, effective)
      .background(ChillReplayPolicyProbe(policy: effective))
  }
}

extension View {
  /// Restricts behavior capture for this subtree. A descendant can only make
  /// the effective policy stricter; it cannot undo an ancestor restriction.
  public func privacy(_ disposition: ViewPrivacyDisposition) -> some View {
    modifier(
      ChillCapturePolicyModifier(privacy: disposition, replay: nil)
    )
  }

  /// Restricts structural replay for this subtree without changing analytics.
  public func replay(_ disposition: ReplayDisposition) -> some View {
    modifier(
      ChillCapturePolicyModifier(privacy: nil, replay: disposition)
    )
  }
}
