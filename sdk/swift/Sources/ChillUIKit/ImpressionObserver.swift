#if os(iOS)
  import ChillCore
  import UIKit

  @MainActor
  final class UIKitImpressionObserver {
    private struct PendingKey: Equatable {
      let declaration: UIKitImpressionDeclaration
      let context: UIKitFactContext
      let surfaceID: SurfaceID
      let elementInstanceID: ElementInstanceID
    }

    weak var view: UIView?
    weak var node: UIKitSemanticNode?
    private var pending: Task<Void, Never>?
    private var pendingKey: PendingKey?
    private var hasEmitted = false
    private var scrollObservation: NSKeyValueObservation?
    private weak var registeredCoordinator: UIKitSurfaceCoordinator?

    init(view: UIView, node: UIKitSemanticNode) {
      self.view = view
      self.node = node
      observeNearestScrollView()
    }

    func evaluateVisibility() {
      guard let view, let node, let declaration = node.impression else {
        becameHidden()
        return
      }
      let (context, surfaceID, _) = resolveUIKitFactContext(for: view)
      guard Chill.isEnabled,
        context.policy.privacy != .blocked,
        context.page?.exposure != .retained,
        context.page?.exposure != .occluded,
        visibilityRatio(of: view) >= declaration.visibilityThreshold,
        declaration.repeats || !hasEmitted
      else {
        becameHidden()
        return
      }
      let key = PendingKey(
        declaration: declaration,
        context: context,
        surfaceID: surfaceID,
        elementInstanceID: node.elementInstanceID
      )
      guard pendingKey != key else { return }
      pending?.cancel()
      pendingKey = key
      pending = Task { @MainActor [weak self] in
        do {
          try await Task.sleep(for: declaration.minimumVisibleDuration)
        } catch {
          return
        }
        guard let self, !Task.isCancelled, pendingKey == key else { return }
        hasEmitted = true
        pending = nil
        pendingKey = nil
        UIKitFactEmitter.impression(
          name: declaration.name,
          role: declaration.role,
          visibilityRatio: declaration.visibilityThreshold,
          visibleDurationNano: declaration.minimumVisibleDuration
            .uiKitImpressionNanoseconds,
          surfaceID: surfaceID,
          elementInstanceID: key.elementInstanceID,
          context: context
        )
      }
    }

    func didMoveToWindow() {
      let nextCoordinator = view?.window.flatMap {
        surfaceCoordinator(for: $0)
      }
      if registeredCoordinator !== nextCoordinator {
        registeredCoordinator?.unregisterImpression(self)
        nextCoordinator?.registerImpression(self)
        registeredCoordinator = nextCoordinator
      }
      observeNearestScrollView()
      evaluateVisibility()
    }

    func resetForReuse() {
      pending?.cancel()
      pending = nil
      pendingKey = nil
      hasEmitted = false
    }

    private func becameHidden() {
      pending?.cancel()
      pending = nil
      pendingKey = nil
    }

    private func observeNearestScrollView() {
      scrollObservation = nil
      guard let scrollView = nearestScrollView() else { return }
      scrollObservation = scrollView.observe(
        \.contentOffset,
        options: [.new]
      ) { [weak self] _, _ in
        Task { @MainActor in self?.evaluateVisibility() }
      }
    }

    private func nearestScrollView() -> UIScrollView? {
      var cursor = view?.superview
      while let current = cursor {
        if let scroll = current as? UIScrollView { return scroll }
        cursor = current.superview
      }
      return nil
    }

    private func visibilityRatio(of view: UIView) -> Double {
      guard let window = view.window,
        !view.isHidden,
        view.alpha > 0.01,
        view.bounds.width.isFinite,
        view.bounds.height.isFinite,
        view.bounds.width > 0,
        view.bounds.height > 0
      else {
        return 0
      }
      let viewFrame = view.convert(view.bounds, to: window)
      var visible = viewFrame.intersection(window.bounds)
      var ancestor = view.superview
      while let current = ancestor {
        guard !current.isHidden, current.alpha > 0.01 else { return 0 }
        if current.clipsToBounds || current is UIScrollView {
          visible = visible.intersection(
            current.convert(current.bounds, to: window)
          )
        }
        ancestor = current.superview
      }
      guard !visible.isNull, !visible.isEmpty else { return 0 }
      let area = viewFrame.width * viewFrame.height
      guard area.isFinite, area > 0 else { return 0 }
      return min(max((visible.width * visible.height) / area, 0), 1)
    }

    deinit {
      pending?.cancel()
    }
  }

  @MainActor
  final class UIKitImpressionProbeView: UIView {
    weak var observer: UIKitImpressionObserver?

    init(observer: UIKitImpressionObserver) {
      self.observer = observer
      super.init(frame: .zero)
      isHidden = true
      isUserInteractionEnabled = false
      isAccessibilityElement = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
      fatalError("init(coder:) is unavailable")
    }

    override func didMoveToWindow() {
      super.didMoveToWindow()
      observer?.didMoveToWindow()
    }

    override func layoutSubviews() {
      super.layoutSubviews()
      observer?.evaluateVisibility()
    }
  }

  @MainActor
  func installUIKitImpressionObserver(
    on view: UIView,
    node: UIKitSemanticNode
  ) {
    let observer: UIKitImpressionObserver
    if let existing = node.impressionObserver {
      observer = existing
    } else {
      observer = UIKitImpressionObserver(view: view, node: node)
      node.impressionObserver = observer
    }
    if node.impressionProbe?.superview !== view {
      node.impressionProbe?.removeFromSuperview()
      let probe = UIKitImpressionProbeView(observer: observer)
      node.impressionProbe = probe
      view.addSubview(probe)
    }
    observer.didMoveToWindow()
  }

  extension Duration {
    fileprivate var uiKitImpressionNanoseconds: UInt64 {
      let components = self.components
      guard components.seconds >= 0 else { return 0 }
      let seconds = UInt64(clamping: components.seconds)
      let attoseconds = max(components.attoseconds, 0)
      let nanos = UInt64(attoseconds / 1_000_000_000)
      let (whole, overflow) = seconds.multipliedReportingOverflow(
        by: 1_000_000_000
      )
      guard !overflow else { return .max }
      let (sum, additionOverflow) = whole.addingReportingOverflow(nanos)
      return additionOverflow ? .max : sum
    }
  }
#endif
