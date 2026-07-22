#if canImport(UIKit) && !os(watchOS)
  import ChillCore
  #if os(iOS)
    import ChillUIKit
  #endif
  import ObjectiveC
  import UIKit

  @MainActor
  package final class UIKitReplayObserver: NSObject,
    ReplayPlatformObserving, UIGestureRecognizerDelegate
  {
    private struct WindowGestures {
      weak var window: UIWindow?
      let tap: UITapGestureRecognizer
      let pan: UIPanGestureRecognizer
    }

    private let admission: ReplayAdmission
    private let interval: TimeInterval
    private let maximumNodes: Int
    private var timer: Timer?
    private var notifications: [NSObjectProtocol] = []
    private var windowGestures: [ObjectIdentifier: WindowGestures] = [:]
    private var wasEnabled = false
    private var consentStateKnown = false
    private var wasConsented = false

    package init(
      admission: ReplayAdmission,
      interval: TimeInterval,
      maximumNodes: Int
    ) {
      self.admission = admission
      self.interval = interval
      self.maximumNodes = maximumNodes
    }

    package func start() {
      guard timer == nil else { return }
      let timer = Timer(timeInterval: interval, repeats: true) {
        [weak self] _ in
        MainActor.assumeIsolated { self?.observe() }
      }
      RunLoop.main.add(timer, forMode: .common)
      self.timer = timer
      let center = NotificationCenter.default
      notifications = [
        center.addObserver(
          forName: UIApplication.didEnterBackgroundNotification,
          object: nil,
          queue: .main
        ) { [weak self] _ in
          Task { @MainActor [weak self] in await self?.flushForLifecycle() }
        },
        center.addObserver(
          forName: UIApplication.willResignActiveNotification,
          object: nil,
          queue: .main
        ) { [weak self] _ in
          Task { @MainActor [weak self] in await self?.flushForLifecycle() }
        },
      ]
      observe()
    }

    package func stop() {
      timer?.invalidate()
      timer = nil
      let center = NotificationCenter.default
      for notification in notifications { center.removeObserver(notification) }
      notifications.removeAll(keepingCapacity: false)
      for item in windowGestures.values {
        if let window = item.window {
          window.removeGestureRecognizer(item.tap)
          window.removeGestureRecognizer(item.pan)
        }
      }
      windowGestures.removeAll(keepingCapacity: false)
    }

    package func gestureRecognizer(
      _: UIGestureRecognizer,
      shouldRecognizeSimultaneouslyWith _: UIGestureRecognizer
    ) -> Bool {
      true
    }

    @objc private func observedTap(_ recognizer: UITapGestureRecognizer) {
      guard recognizer.state == .ended else { return }
      emitGesture(from: recognizer, kind: .pointer)
    }

    @objc private func observedPan(_ recognizer: UIPanGestureRecognizer) {
      emitGesture(from: recognizer, kind: .drag)
    }

    private func observe() {
      guard let runtime = Chill.currentRuntime() else {
        if wasEnabled {
          wasEnabled = false
          Task { [admission] in await admission.discardUnsealed() }
        }
        return
      }
      let consented = runtime.isCaptureEnabled(for: .replay)
      if !consented {
        if !consentStateKnown || wasConsented {
          Task { [admission] in try? await admission.revokeConsent() }
        }
        consentStateKnown = true
        wasConsented = false
        wasEnabled = false
        return
      }
      consentStateKnown = true
      wasConsented = true
      guard runtime.isCaptureEnabled(for: .replay) else {
        if wasEnabled {
          wasEnabled = false
          Task { [admission] in await admission.discardUnsealed() }
        }
        return
      }
      wasEnabled = true
      let windows = activeWindows()
      installGestureObservers(on: windows)
      guard
        let observation = UIKitReplayTree.snapshot(
          windows: windows,
          runtime: runtime,
          maximumNodes: maximumNodes
        )
      else {
        return
      }
      admission.submit(observation)
    }

    private func emitGesture(
      from recognizer: UIGestureRecognizer,
      kind: ReplayGestureKind
    ) {
      guard let runtime = Chill.currentRuntime(),
        runtime.isCaptureEnabled(for: .replay),
        let window = recognizer.view as? UIWindow
      else { return }
      let location = recognizer.location(in: window)
      let target = window.hitTest(location, with: nil)
      guard target.map(UIKitReplayTree.isEligibleForReplay) ?? true else {
        return
      }
      let phase: ReplayGesturePhase =
        switch recognizer.state {
        case .began: .began
        case .changed: .changed
        case .cancelled, .failed: .cancelled
        default: .ended
        }
      let time = runtime.timePoint()
      admission.submit(
        TimedReplayGesture(
          gesture: ReplayGesture(
            kind: kind,
            phase: phase,
            location: ReplayPoint(
              x: location.x,
              y: location.y
            ),
            targetNodeID: target.flatMap(UIKitReplayTree.gestureTargetNodeID)
          ),
          occurredAtUnixNano: time.occurredAtUnixNano,
          monotonicNano: time.monotonicNano
        )
      )
    }

    private func activeWindows() -> [UIWindow] {
      UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }
        .filter { $0.activationState != .unattached }
        .flatMap(\.windows)
        .filter { !$0.isHidden && $0.alpha > 0 && !$0.bounds.isEmpty }
        .sorted { left, right in
          if left.windowLevel != right.windowLevel {
            return left.windowLevel.rawValue < right.windowLevel.rawValue
          }
          return ObjectIdentifier(left).hashValue
            < ObjectIdentifier(right).hashValue
        }
    }

    private func installGestureObservers(on windows: [UIWindow]) {
      let active = Set(windows.map(ObjectIdentifier.init))
      for (id, item) in windowGestures where !active.contains(id) {
        if let window = item.window {
          window.removeGestureRecognizer(item.tap)
          window.removeGestureRecognizer(item.pan)
        }
        windowGestures.removeValue(forKey: id)
      }
      for window in windows {
        let id = ObjectIdentifier(window)
        guard windowGestures[id] == nil else { continue }
        let tap = UITapGestureRecognizer(
          target: self,
          action: #selector(observedTap(_:))
        )
        tap.cancelsTouchesInView = false
        tap.delaysTouchesBegan = false
        tap.delaysTouchesEnded = false
        tap.delegate = self
        let pan = UIPanGestureRecognizer(
          target: self,
          action: #selector(observedPan(_:))
        )
        pan.cancelsTouchesInView = false
        pan.delaysTouchesBegan = false
        pan.delaysTouchesEnded = false
        pan.delegate = self
        window.addGestureRecognizer(tap)
        window.addGestureRecognizer(pan)
        windowGestures[id] = WindowGestures(
          window: window,
          tap: tap,
          pan: pan
        )
      }
    }

    private func flushForLifecycle() async {
      await admission.flush()
    }
  }

  @MainActor
  package enum UIKitReplayTree {
    private struct AssociatedKeys {
      nonisolated(unsafe) static var replayNodeID: UInt8 = 0
    }

    private struct Metadata {
      let semanticID: String?
      let semanticRole: String?
      let policy: ViewCapturePolicy
    }

    private struct PolicyRegion {
      let rect: CGRect
      let policy: ViewCapturePolicy
    }

    package static func snapshot(
      windows: [UIWindow],
      runtime: ChillRuntime,
      maximumNodes: Int
    ) -> ReplayObservation? {
      guard let primary = windows.last else { return nil }
      var nodes: [ReplayNode] = []
      for (index, window) in windows.enumerated() where nodes.count < maximumNodes {
        let policyRegions = policyRegions(in: window, window: window)
        visit(
          window,
          window: window,
          parentID: nil,
          siblingIndex: index,
          inheritedPolicy: .standard,
          inheritedVisible: true,
          inheritedOpacity: 1,
          policyRegions: policyRegions,
          maximumNodes: maximumNodes,
          nodes: &nodes
        )
      }
      let time = runtime.timePoint()
      let insets = primary.safeAreaInsets
      return ReplayObservation(
        sessionID: runtime.configuration.sessionID.rawValue,
        bootID: runtime.configuration.bootID.rawValue,
        occurredAtUnixNano: time.occurredAtUnixNano,
        monotonicNano: time.monotonicNano,
        viewport: ReplayViewport(
          size: ReplaySize(
            width: primary.bounds.width,
            height: primary.bounds.height
          ),
          safeAreaTop: insets.top,
          safeAreaLeft: insets.left,
          safeAreaBottom: insets.bottom,
          safeAreaRight: insets.right
        ),
        nodes: nodes
      )
    }

    package static func nodeID(_ view: UIView) -> String {
      if let existing = objc_getAssociatedObject(
        view,
        &AssociatedKeys.replayNodeID
      ) as? NSString {
        return existing as String
      }
      let created = UUIDv7.generate().uuidString.lowercased() as NSString
      objc_setAssociatedObject(
        view,
        &AssociatedKeys.replayNodeID,
        created,
        .OBJC_ASSOCIATION_RETAIN_NONATOMIC
      )
      return created as String
    }

    package static func isEligibleForReplay(_ view: UIView) -> Bool {
      var policy = ViewCapturePolicy.standard
      var cursor: UIView? = view
      while let current = cursor {
        let local = metadata(for: current).policy
        policy = policy.restricting(
          privacy: local.privacy,
          replay: local.replay
        )
        cursor = current.superview
      }
      if let window = view.window {
        let rect = view.convert(view.bounds, to: window).standardized
        for region in policyRegions(in: window, window: window)
        where contains(region.rect, rect) {
          policy = policy.restricting(
            privacy: region.policy.privacy,
            replay: region.policy.replay
          )
        }
      }
      return policy.privacy != .blocked && policy.replay != .blocked
    }

    package static func gestureTargetNodeID(_ view: UIView) -> String? {
      guard isEligibleForReplay(view) else { return nil }
      var cursor: UIView? = view
      while let current = cursor {
        if !classify(current, forceMask: false).traverseChildren {
          return nodeID(current)
        }
        cursor = current.superview
      }
      return nodeID(view)
    }

    private static func visit(
      _ view: UIView,
      window: UIWindow,
      parentID: String?,
      siblingIndex: Int,
      inheritedPolicy: ViewCapturePolicy,
      inheritedVisible: Bool,
      inheritedOpacity: Double,
      policyRegions: [PolicyRegion],
      maximumNodes: Int,
      nodes: inout [ReplayNode]
    ) {
      guard nodes.count < maximumNodes,
        !(view is any ReplayPolicyBoundary)
      else { return }
      let local = metadata(for: view)
      let converted = view.convert(view.bounds, to: window).standardized
      var policy = inheritedPolicy.restricting(
        privacy: local.policy.privacy,
        replay: local.policy.replay
      )
      for region in policyRegions where contains(region.rect, converted) {
        policy = policy.restricting(
          privacy: region.policy.privacy,
          replay: region.policy.replay
        )
      }
      guard policy.privacy != .blocked, policy.replay != .blocked else {
        return
      }
      let classification = classify(view, forceMask: policy.replay == .masked)
      let id = nodeID(view)
      let opacity = inheritedOpacity * Double(view.alpha)
      let visible =
        inheritedVisible && !view.isHidden && opacity > 0.01
        && !converted.isEmpty && converted.intersects(window.bounds)
      let control = view as? UIControl
      let scroll = view as? UIScrollView
      nodes.append(
        ReplayNode(
          id: id,
          parentID: parentID,
          siblingIndex: siblingIndex,
          role: local.semanticRole.flatMap(ReplayRole.init(rawValue:))
            ?? classification.role,
          frame: ReplayRect(
            x: converted.origin.x,
            y: converted.origin.y,
            width: converted.width,
            height: converted.height
          ),
          visible: visible,
          enabled: control?.isEnabled ?? view.isUserInteractionEnabled,
          selected: control?.isSelected ?? false,
          focused: view.isFirstResponder,
          opacity: opacity,
          semanticID: local.semanticID,
          contentMask: classification.mask,
          scroll: scroll.map {
            ReplayScrollState(
              offset: ReplayPoint(
                x: $0.contentOffset.x,
                y: $0.contentOffset.y
              ),
              contentSize: ReplaySize(
                width: $0.contentSize.width,
                height: $0.contentSize.height
              )
            )
          }
        )
      )
      guard classification.traverseChildren else { return }
      for (index, child) in view.subviews.enumerated() {
        visit(
          child,
          window: window,
          parentID: id,
          siblingIndex: index,
          inheritedPolicy: policy,
          inheritedVisible: visible,
          inheritedOpacity: opacity,
          policyRegions: policyRegions,
          maximumNodes: maximumNodes,
          nodes: &nodes
        )
      }
    }

    private static func policyRegions(
      in view: UIView,
      window: UIWindow
    ) -> [PolicyRegion] {
      var result: [PolicyRegion] = []
      gatherPolicyRegions(in: view, window: window, result: &result)
      return result
    }

    private static func gatherPolicyRegions(
      in view: UIView,
      window: UIWindow,
      result: inout [PolicyRegion]
    ) {
      if let boundary = view as? any ReplayPolicyBoundary {
        let rect = view.convert(view.bounds, to: window).standardized
        if !rect.isEmpty {
          result.append(
            PolicyRegion(rect: rect, policy: boundary.chillReplayPolicy)
          )
        }
        return
      }
      for child in view.subviews {
        gatherPolicyRegions(in: child, window: window, result: &result)
      }
    }

    private static func contains(_ outer: CGRect, _ inner: CGRect) -> Bool {
      outer.insetBy(dx: -0.5, dy: -0.5).contains(inner)
    }

    private static func metadata(for view: UIView) -> Metadata {
      #if os(iOS)
        let value = replayMetadata(for: view)
        return Metadata(
          semanticID: value.semanticID,
          semanticRole: value.semanticRole,
          policy: value.policy
        )
      #else
        return Metadata(
          semanticID: nil,
          semanticRole: nil,
          policy: .standard
        )
      #endif
    }

    private static func classify(
      _ view: UIView,
      forceMask: Bool
    ) -> (role: ReplayRole, mask: ReplayContentMask?, traverseChildren: Bool) {
      if let field = view as? UITextField {
        if field.isSecureTextEntry {
          return (.textInput, .secureInput, true)
        }
        return (.textInput, .text(length: field.text?.count ?? 0), true)
      }
      if let text = view as? UITextView {
        return (.textInput, .text(length: text.text.count), true)
      }
      if let label = view as? UILabel {
        let count = label.text?.count ?? label.attributedText?.length ?? 0
        return (.text, .text(length: count), true)
      }
      if let button = view as? UIButton {
        let count =
          button.title(for: .normal)?.count
          ?? button.configuration?.title?.count ?? 0
        return (.button, .text(length: count), true)
      }
      #if !os(tvOS)
        if view is UISwitch { return (.toggle, nil, true) }
        if view is UISlider || view is UIStepper {
          return (.slider, nil, true)
        }
        if view is UIPickerView || view is UIDatePicker {
          return (.picker, nil, true)
        }
      #endif
      if view is UIImageView { return (.image, .pixels, false) }
      if view is UIScrollView { return (.scroll, nil, true) }
      let className = NSStringFromClass(type(of: view))
      if className.contains("WKWebView") {
        return (.web, .pixels, false)
      }
      if className.contains("Player") || className.contains("Video") {
        return (.media, .pixels, false)
      }
      if forceMask { return (.container, .customDrawing, false) }
      let bundleID = Bundle(for: type(of: view)).bundleIdentifier ?? ""
      let platformOwned = bundleID.hasPrefix("com.apple.")
      if type(of: view) == UIView.self || view is UIStackView || platformOwned {
        return (.container, nil, true)
      }
      return (.custom, .customDrawing, false)
    }
  }
#endif
