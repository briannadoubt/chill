#if canImport(AppKit) && !targetEnvironment(macCatalyst)
  import AppKit
  import ChillCore
  import ObjectiveC

  @MainActor
  package final class AppKitReplayObserver: ReplayPlatformObserving {
    private let admission: ReplayAdmission
    private let interval: TimeInterval
    private let maximumNodes: Int
    private var timer: Timer?
    private var notifications: [NSObjectProtocol] = []
    private var eventMonitor: Any?
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
          forName: NSApplication.didResignActiveNotification,
          object: nil,
          queue: .main
        ) { [weak self] _ in
          Task { @MainActor [weak self] in await self?.flushForLifecycle() }
        },
        center.addObserver(
          forName: NSApplication.willTerminateNotification,
          object: nil,
          queue: .main
        ) { [weak self] _ in
          Task { @MainActor [weak self] in await self?.flushForLifecycle() }
        },
      ]
      eventMonitor = NSEvent.addLocalMonitorForEvents(
        matching: [.leftMouseDown, .leftMouseDragged, .leftMouseUp, .scrollWheel]
      ) { [weak self] event in
        MainActor.assumeIsolated { self?.observe(event) }
        return event
      }
      observe()
    }

    package func stop() {
      timer?.invalidate()
      timer = nil
      let center = NotificationCenter.default
      for notification in notifications { center.removeObserver(notification) }
      notifications.removeAll(keepingCapacity: false)
      if let eventMonitor { NSEvent.removeMonitor(eventMonitor) }
      eventMonitor = nil
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
      guard
        let observation = AppKitReplayTree.snapshot(
          windows: activeWindows(),
          runtime: runtime,
          maximumNodes: maximumNodes
        )
      else { return }
      admission.submit(observation)
    }

    private func observe(_ event: NSEvent) {
      guard let runtime = Chill.currentRuntime(),
        runtime.isCaptureEnabled(for: .replay),
        let window = event.window
      else { return }
      let target = window.contentView?.hitTest(event.locationInWindow)
      guard target.map(AppKitReplayTree.isEligibleForReplay) ?? true else {
        return
      }
      let kind: ReplayGestureKind =
        event.type == .scrollWheel
        ? .scroll
        : event.type == .leftMouseDragged ? .drag : .pointer
      let phase: ReplayGesturePhase =
        switch event.type {
        case .leftMouseDown: .began
        case .leftMouseDragged: .changed
        case .leftMouseUp: .ended
        case .scrollWheel:
          switch event.phase {
          case .began: .began
          case .changed: .changed
          case .cancelled: .cancelled
          default: .ended
          }
        default: .ended
        }
      let time = runtime.timePoint()
      admission.submit(
        TimedReplayGesture(
          gesture: ReplayGesture(
            kind: kind,
            phase: phase,
            location: ReplayPoint(
              x: event.locationInWindow.x,
              y: event.locationInWindow.y
            ),
            targetNodeID: target.flatMap(AppKitReplayTree.gestureTargetNodeID)
          ),
          occurredAtUnixNano: time.occurredAtUnixNano,
          monotonicNano: time.monotonicNano
        )
      )
    }

    private func activeWindows() -> [NSWindow] {
      NSApplication.shared.windows
        .filter { $0.isVisible && !$0.isMiniaturized && $0.contentView != nil }
        .sorted { left, right in
          if left.level != right.level { return left.level.rawValue < right.level.rawValue }
          return left.windowNumber < right.windowNumber
        }
    }

    private func flushForLifecycle() async {
      await admission.flush()
    }
  }

  @MainActor
  package enum AppKitReplayTree {
    private struct AssociatedKeys {
      nonisolated(unsafe) static var replayNodeID: UInt8 = 0
    }

    private struct PolicyRegion {
      let rect: CGRect
      let policy: ViewCapturePolicy
    }

    package static func snapshot(
      windows: [NSWindow],
      runtime: ChillRuntime,
      maximumNodes: Int
    ) -> ReplayObservation? {
      guard let primary = windows.last, let primaryContent = primary.contentView
      else { return nil }
      var nodes: [ReplayNode] = []
      for (index, window) in windows.enumerated() where nodes.count < maximumNodes {
        guard let content = window.contentView else { continue }
        let policyRegions = policyRegions(in: content, root: content)
        visit(
          content,
          root: content,
          parentID: nil,
          siblingIndex: index,
          inheritedVisible: true,
          inheritedOpacity: 1,
          inheritedPolicy: .standard,
          policyRegions: policyRegions,
          maximumNodes: maximumNodes,
          nodes: &nodes
        )
      }
      let time = runtime.timePoint()
      return ReplayObservation(
        sessionID: runtime.configuration.sessionID.rawValue,
        bootID: runtime.configuration.bootID.rawValue,
        occurredAtUnixNano: time.occurredAtUnixNano,
        monotonicNano: time.monotonicNano,
        viewport: ReplayViewport(
          size: ReplaySize(
            width: primaryContent.bounds.width,
            height: primaryContent.bounds.height
          )
        ),
        nodes: nodes
      )
    }

    package static func nodeID(_ view: NSView) -> String {
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

    package static func isEligibleForReplay(_ view: NSView) -> Bool {
      guard let root = view.window?.contentView else { return true }
      let rect = view.convert(view.bounds, to: root)
      var policy = ViewCapturePolicy.standard
      for region in policyRegions(in: root, root: root)
      where contains(region.rect, rect) {
        policy = policy.restricting(
          privacy: region.policy.privacy,
          replay: region.policy.replay
        )
      }
      return policy.privacy != .blocked && policy.replay != .blocked
    }

    package static func gestureTargetNodeID(_ view: NSView) -> String? {
      guard isEligibleForReplay(view) else { return nil }
      var cursor: NSView? = view
      while let current = cursor {
        if !classify(current).traverseChildren { return nodeID(current) }
        cursor = current.superview
      }
      return nodeID(view)
    }

    private static func visit(
      _ view: NSView,
      root: NSView,
      parentID: String?,
      siblingIndex: Int,
      inheritedVisible: Bool,
      inheritedOpacity: Double,
      inheritedPolicy: ViewCapturePolicy,
      policyRegions: [PolicyRegion],
      maximumNodes: Int,
      nodes: inout [ReplayNode]
    ) {
      guard nodes.count < maximumNodes,
        !(view is any ReplayPolicyBoundary)
      else { return }
      let converted = view.convert(view.bounds, to: root)
      var policy = inheritedPolicy
      for region in policyRegions where contains(region.rect, converted) {
        policy = policy.restricting(
          privacy: region.policy.privacy,
          replay: region.policy.replay
        )
      }
      guard policy.privacy != .blocked, policy.replay != .blocked else {
        return
      }
      let classification = classify(
        view,
        forceMask: policy.replay == .masked
      )
      let id = nodeID(view)
      let opacity = inheritedOpacity * Double(view.alphaValue)
      let visible =
        inheritedVisible && !view.isHidden && opacity > 0.01
        && !converted.isEmpty && converted.intersects(root.bounds)
      let control = view as? NSControl
      let scroll = view as? NSScrollView
      nodes.append(
        ReplayNode(
          id: id,
          parentID: parentID,
          siblingIndex: siblingIndex,
          role: classification.role,
          frame: ReplayRect(
            x: converted.origin.x,
            y: converted.origin.y,
            width: converted.width,
            height: converted.height
          ),
          visible: visible,
          enabled: control?.isEnabled ?? true,
          selected: (control as? NSButton)?.state == .on,
          focused: view.window?.firstResponder === view,
          opacity: opacity,
          contentMask: classification.mask,
          scroll: scroll.map {
            ReplayScrollState(
              offset: ReplayPoint(
                x: $0.contentView.bounds.origin.x,
                y: $0.contentView.bounds.origin.y
              ),
              contentSize: ReplaySize(
                width: Double($0.documentView?.bounds.width ?? 0),
                height: Double($0.documentView?.bounds.height ?? 0)
              )
            )
          }
        )
      )
      guard classification.traverseChildren else { return }
      for (index, child) in view.subviews.enumerated() {
        visit(
          child,
          root: root,
          parentID: id,
          siblingIndex: index,
          inheritedVisible: visible,
          inheritedOpacity: opacity,
          inheritedPolicy: policy,
          policyRegions: policyRegions,
          maximumNodes: maximumNodes,
          nodes: &nodes
        )
      }
    }

    private static func policyRegions(
      in view: NSView,
      root: NSView
    ) -> [PolicyRegion] {
      var result: [PolicyRegion] = []
      gatherPolicyRegions(in: view, root: root, result: &result)
      return result
    }

    private static func gatherPolicyRegions(
      in view: NSView,
      root: NSView,
      result: inout [PolicyRegion]
    ) {
      if let boundary = view as? any ReplayPolicyBoundary {
        let rect = view.convert(view.bounds, to: root)
        if !rect.isEmpty {
          result.append(
            PolicyRegion(rect: rect, policy: boundary.chillReplayPolicy)
          )
        }
        return
      }
      for child in view.subviews {
        gatherPolicyRegions(in: child, root: root, result: &result)
      }
    }

    private static func contains(_ outer: CGRect, _ inner: CGRect) -> Bool {
      outer.insetBy(dx: -0.5, dy: -0.5).contains(inner)
    }

    private static func classify(
      _ view: NSView,
      forceMask: Bool = false
    ) -> (role: ReplayRole, mask: ReplayContentMask?, traverseChildren: Bool) {
      if view is NSSecureTextField {
        return (.textInput, .secureInput, true)
      }
      if let field = view as? NSTextField {
        let role: ReplayRole = field.isEditable ? .textInput : .text
        return (role, .text(length: field.stringValue.count), true)
      }
      if let button = view as? NSButton {
        return (.button, .text(length: button.title.count), true)
      }
      if view is NSSlider || view is NSStepper {
        return (.slider, nil, true)
      }
      if view is NSPopUpButton || view is NSComboBox {
        return (.picker, nil, true)
      }
      if view is NSImageView { return (.image, .pixels, false) }
      if view is NSScrollView { return (.scroll, nil, true) }
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
      if type(of: view) == NSView.self || platformOwned {
        return (.container, nil, true)
      }
      return (.custom, .customDrawing, false)
    }
  }
#endif
