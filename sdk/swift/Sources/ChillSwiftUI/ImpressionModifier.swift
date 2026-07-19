import ChillCore
import SwiftUI

public struct ImpressionPolicy: Equatable, Sendable {
  public let visibilityThreshold: Double
  public let minimumVisibleDuration: Duration
  public let repeats: Bool

  public static let standard = ImpressionPolicy()

  public init(
    visibilityThreshold: Double = 0.5,
    minimumVisibleDuration: Duration = .milliseconds(500),
    repeats: Bool = false
  ) {
    self.visibilityThreshold =
      visibilityThreshold.isFinite
      ? min(max(visibilityThreshold, 0), 1)
      : 1
    self.minimumVisibleDuration = max(minimumVisibleDuration, .zero)
    self.repeats = repeats
  }
}

@MainActor
final class ImpressionObserver {
  private var pending: Task<Void, Never>?
  private var hasEmitted = false

  func becameVisible(
    policy: ImpressionPolicy,
    emit: @escaping @MainActor (UInt64) -> Void
  ) {
    guard policy.repeats || !hasEmitted else { return }
    pending?.cancel()
    pending = Task { @MainActor in
      do {
        try await Task.sleep(for: policy.minimumVisibleDuration)
      } catch {
        return
      }
      guard !Task.isCancelled else { return }
      hasEmitted = true
      emit(policy.minimumVisibleDuration.clampedNanoseconds)
    }
  }

  func becameHidden() {
    pending?.cancel()
    pending = nil
  }

  deinit {
    pending?.cancel()
  }
}

private struct ChillImpressionModifier: ViewModifier {
  @Environment(\.chillAnnotationContext) private var annotations
  @Environment(\.chillCapturePolicy) private var capturePolicy
  @Environment(\.chillPageAncestry) private var page
  @State private var elementInstanceID = newElementInstanceID()
  @State private var fallbackSurfaceID = newSurfaceID()
  @State private var observer = ImpressionObserver()
  @State private var geometricallyVisible = false

  let name: SemanticName?
  let role: SemanticName?
  let policy: ImpressionPolicy

  func body(content: Content) -> some View {
    let eligible = ImpressionEligibility(
      enabled: Chill.isEnabled,
      privacy: capturePolicy.privacy,
      pageExposure: page?.path.exposure
    ).allowsCapture
    let threshold = policy.visibilityThreshold
    content
      .onGeometryChange(for: Bool.self) { proxy in
        visibilityThresholdReached(
          viewBounds: CGRect(origin: .zero, size: proxy.size),
          viewportBounds: proxy.bounds(of: .scrollView),
          threshold: threshold
        )
      } action: { visible in
        geometricallyVisible = visible
        synchronize(eligible: eligible)
      }
      .onChange(of: eligible) { _, eligible in
        synchronize(eligible: eligible)
      }
      .onDisappear {
        geometricallyVisible = false
        observer.becameHidden()
      }
  }

  private func synchronize(eligible: Bool) {
    guard eligible, geometricallyVisible else {
      observer.becameHidden()
      return
    }
    becameVisible()
  }

  private func becameVisible() {
    guard let name,
      let role
    else {
      return
    }
    let context = SwiftUIFactContext(
      annotations: annotations.snapshot,
      page: page?.path,
      policy: capturePolicy
    )
    let surfaceID = page?.path.surfaceID ?? fallbackSurfaceID
    let elementInstanceID = elementInstanceID
    let visibilityThreshold = policy.visibilityThreshold
    observer.becameVisible(policy: policy) { duration in
      ChillUIInstrumentation.impression(
        name: name,
        role: role,
        visibilityRatio: visibilityThreshold,
        visibleDurationNano: duration,
        surfaceID: surfaceID,
        elementInstanceID: elementInstanceID,
        context: context
      )
    }
  }
}

private struct ImpressionEligibility: Equatable {
  let enabled: Bool
  let privacy: ViewPrivacyDisposition
  let pageExposure: PageExposure?

  var allowsCapture: Bool {
    guard enabled, privacy != .blocked else { return false }
    return pageExposure != .retained && pageExposure != .occluded
  }
}

func visibilityThresholdReached(
  viewBounds: CGRect,
  viewportBounds: CGRect?,
  threshold: Double
) -> Bool {
  guard viewBounds.width.isFinite,
    viewBounds.height.isFinite,
    viewBounds.width > 0,
    viewBounds.height > 0
  else {
    return false
  }
  guard let viewportBounds else { return true }
  let intersection = viewBounds.intersection(viewportBounds)
  guard !intersection.isNull, !intersection.isEmpty else { return false }
  let area = viewBounds.width * viewBounds.height
  let visibleArea = intersection.width * intersection.height
  return visibleArea / area >= threshold
}

extension View {
  /// Emits one bounded impression after the declared view remains visibly
  /// above its threshold for the policy duration. The default never captures
  /// text, pixels, or labels.
  public func impression(
    _ name: String,
    role: String,
    policy: ImpressionPolicy = .standard
  ) -> some View {
    modifier(
      ChillImpressionModifier(
        name: try? SemanticName(name),
        role: try? SemanticName(role),
        policy: policy
      )
    )
  }
}

extension Duration {
  var clampedNanoseconds: UInt64 {
    let components = self.components
    guard components.seconds >= 0 else { return 0 }
    let seconds = UInt64(components.seconds)
    let nanosFromSeconds = seconds.multipliedReportingOverflow(
      by: 1_000_000_000
    )
    guard !nanosFromSeconds.overflow else { return .max }
    let attoseconds = max(components.attoseconds, 0)
    let nanos = UInt64(attoseconds / 1_000_000_000)
    let total = nanosFromSeconds.partialValue.addingReportingOverflow(nanos)
    return total.overflow ? .max : total.partialValue
  }
}
