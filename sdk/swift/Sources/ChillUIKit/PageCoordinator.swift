#if os(iOS)
  import ChillCore
  import UIKit

  @MainActor
  final class UIKitSurfaceCoordinator: NSObject {
    private static let nativeActivationLimit = 128

    private final class WeakImpressionObserver {
      weak var value: UIKitImpressionObserver?

      init(_ value: UIKitImpressionObserver) {
        self.value = value
      }
    }

    private struct Candidate {
      let state: UIKitPageState
      let parent: UIKitPageState?
      let relation: PageRelation
      let exposure: PageExposure
      let focused: Bool
      let contextNodes: [UIKitSemanticNode]
      let depth: Int
    }

    private struct WalkResult {
      let tailPage: UIKitPageState?
      let tailContext: [UIKitSemanticNode]
      let tailDepth: Int
    }

    weak var window: UIWindow?
    let surfaceID = newUIKitSurfaceID()
    private let graph = UIKitPageSurfaceGraph()
    private var active: [ObjectIdentifier: UIKitPageState] = [:]
    private var reconciliationPending = false
    private var observedScene: UIWindowScene?
    private var lastSceneState: UIScene.ActivationState?
    private var impressionObservers: [ObjectIdentifier: WeakImpressionObserver] = [:]
    private var nativeActivationOrder: [UIKitNativeActivationToken] = []
    private var nativeActivations: Set<UIKitNativeActivationToken> = []

    init(window: UIWindow) {
      self.window = window
      super.init()
      observeSceneIfNeeded()
    }

    func requestReconciliation() {
      guard !reconciliationPending else { return }
      reconciliationPending = true
      Task { @MainActor [weak self] in
        await Task.yield()
        guard let self else { return }
        reconciliationPending = false
        reconcile()
      }
    }

    func reconcile() {
      guard let window, let root = window.rootViewController else {
        finishAllPages()
        return
      }
      observeSceneIfNeeded()
      let sceneIsActive =
        window.windowScene?.activationState
        == .foregroundActive
      let sceneTransitioned =
        window.windowScene?.activationState != lastSceneState
        && lastSceneState != nil
      var candidates: [Candidate] = []
      var visited: Set<ObjectIdentifier> = []
      _ = walk(
        root,
        parentPage: nil,
        inheritedContext: [],
        relationHint: .root,
        branchVisible: true,
        sceneIsActive: sceneIsActive,
        pageDepth: 0,
        window: window,
        visited: &visited,
        candidates: &candidates
      )

      let candidateIDs = Set(
        candidates.map {
          ObjectIdentifier($0.state)
        })
      let removed = active.values.filter {
        !candidateIDs.contains(ObjectIdentifier($0))
      }.sorted { $0.depth > $1.depth }
      for state in removed {
        graph.remove(state.instanceID)
        active.removeValue(forKey: ObjectIdentifier(state))
        state.active = false
        state.lifetime.finish()
        state.contextNodes = []
      }

      var paths: [ObjectIdentifier: PagePath] = [:]
      var newStates: Set<ObjectIdentifier> = []
      for candidate in candidates {
        let identifier = ObjectIdentifier(candidate.state)
        if active[identifier] == nil {
          candidate.state.prepareForInsertion()
          newStates.insert(identifier)
        }
        let parentPath = candidate.parent.flatMap {
          paths[ObjectIdentifier($0)]
        }
        let relation: PageRelation =
          parentPath == nil
          ? .root : candidate.relation
        guard
          let path = try? PagePath(
            surfaceID: surfaceID,
            instanceID: candidate.state.instanceID,
            instanceIDs: (parentPath?.instanceIDs ?? [])
              + [candidate.state.instanceID],
            segments: (parentPath?.segments ?? [])
              + [candidate.state.declaration.segment],
            relation: relation,
            exposure: candidate.exposure,
            focused: candidate.focused && candidate.exposure == .foreground
          )
        else {
          continue
        }
        paths[identifier] = path
        candidate.state.active = true
        candidate.state.depth = candidate.depth
        candidate.state.contextNodes = candidate.contextNodes
        active[identifier] = candidate.state
        graph.register(
          path: path,
          parentInstanceID: parentPath?.instanceID,
          declaredExposure: candidate.exposure,
          declaredFocused: candidate.focused
        )
      }

      for candidate in candidates {
        let identifier = ObjectIdentifier(candidate.state)
        guard let declaredPath = paths[identifier] else { continue }
        let context = resolveContext(nodes: candidate.contextNodes)
        let fact = UIKitPageFact(
          path: graph.resolve(declaredPath),
          annotations: context.annotations,
          policy: context.policy
        )
        if newStates.contains(identifier) {
          let initialCause = initialCause(for: candidate)
          candidate.state.lifetime.appear(fact, initialCause: initialCause)
        } else {
          candidate.state.lifetime.update(
            fact,
            cause: updateCause(
              for: candidate,
              fact: fact,
              sceneTransitioned: sceneTransitioned
            )
          )
        }
      }

      emitSceneLifecycleIfNeeded()
      evaluateImpressions()
    }

    private func walk(
      _ controller: UIViewController,
      parentPage: UIKitPageState?,
      inheritedContext: [UIKitSemanticNode],
      relationHint: PageRelation,
      branchVisible: Bool,
      sceneIsActive: Bool,
      pageDepth: Int,
      window: UIWindow,
      visited: inout Set<ObjectIdentifier>,
      candidates: inout [Candidate]
    ) -> WalkResult {
      let controllerID = ObjectIdentifier(controller)
      guard visited.insert(controllerID).inserted else {
        return WalkResult(
          tailPage: parentPage,
          tailContext: inheritedContext,
          tailDepth: pageDepth
        )
      }

      var context = inheritedContext
      let node = semanticNode(for: controller, create: false)
      if let node { context.append(node) }
      var currentPage = parentPage
      var currentDepth = pageDepth
      if let state = node?.pageState {
        let relation =
          parentPage == nil
          ? PageRelation.root
          : state.declaration.relation ?? relationHint
        let exposure: PageExposure =
          branchVisible && sceneIsActive
          ? .foreground : .retained
        candidates.append(
          Candidate(
            state: state,
            parent: parentPage,
            relation: relation,
            exposure: exposure,
            focused: branchVisible && sceneIsActive,
            contextNodes: context,
            depth: pageDepth
          )
        )
        currentPage = state
        currentDepth = pageDepth + 1
      }

      var activeTail = WalkResult(
        tailPage: currentPage,
        tailContext: context,
        tailDepth: currentDepth
      )
      if let navigation = controller as? UINavigationController {
        var stackPage = currentPage
        var stackContext = context
        var stackDepth = currentDepth
        let lastIndex = navigation.viewControllers.indices.last
        for (index, child) in navigation.viewControllers.enumerated() {
          let result = walk(
            child,
            parentPage: stackPage,
            inheritedContext: stackContext,
            relationHint: .push,
            branchVisible: branchVisible && index == lastIndex,
            sceneIsActive: sceneIsActive,
            pageDepth: stackDepth,
            window: window,
            visited: &visited,
            candidates: &candidates
          )
          stackPage = result.tailPage
          stackContext = result.tailContext
          stackDepth = result.tailDepth
          if index == lastIndex { activeTail = result }
        }
      } else if let tabs = controller as? UITabBarController {
        for child in tabs.viewControllers ?? [] {
          let selected = child === tabs.selectedViewController
          let result = walk(
            child,
            parentPage: currentPage,
            inheritedContext: context,
            relationHint: .tab,
            branchVisible: branchVisible && selected,
            sceneIsActive: sceneIsActive,
            pageDepth: currentDepth,
            window: window,
            visited: &visited,
            candidates: &candidates
          )
          if selected { activeTail = result }
        }
      } else if let split = controller as? UISplitViewController {
        for child in split.viewControllers {
          let displayed = controllerIsDisplayed(child, in: window)
          let result = walk(
            child,
            parentPage: currentPage,
            inheritedContext: context,
            relationHint: .split,
            branchVisible: branchVisible && displayed,
            sceneIsActive: sceneIsActive,
            pageDepth: currentDepth,
            window: window,
            visited: &visited,
            candidates: &candidates
          )
          if displayed { activeTail = result }
        }
      } else {
        let children = controller.children
        for child in children {
          let displayed = controllerIsDisplayed(child, in: window)
          let result = walk(
            child,
            parentPage: currentPage,
            inheritedContext: context,
            relationHint: children.count > 1 ? .split : .push,
            branchVisible: branchVisible && displayed,
            sceneIsActive: sceneIsActive,
            pageDepth: currentDepth,
            window: window,
            visited: &visited,
            candidates: &candidates
          )
          if displayed { activeTail = result }
        }
      }

      if let presented = controller.presentedViewController,
        !presented.isBeingDismissed
      {
        activeTail = walk(
          presented,
          parentPage: activeTail.tailPage,
          inheritedContext: activeTail.tailContext,
          relationHint: presentationRelation(for: presented),
          branchVisible: branchVisible,
          sceneIsActive: sceneIsActive,
          pageDepth: activeTail.tailDepth,
          window: window,
          visited: &visited,
          candidates: &candidates
        )
      }
      return activeTail
    }

    private func controllerIsDisplayed(
      _ controller: UIViewController,
      in window: UIWindow
    ) -> Bool {
      guard let view = controller.viewIfLoaded,
        view.window === window,
        !view.isHidden,
        view.alpha > 0.01,
        !view.bounds.isEmpty
      else {
        return false
      }
      let frame = view.convert(view.bounds, to: window)
      return !frame.intersection(window.bounds).isNull
        && !frame.intersection(window.bounds).isEmpty
    }

    private func presentationRelation(
      for controller: UIViewController
    ) -> PageRelation {
      switch controller.modalPresentationStyle {
      case .fullScreen, .overFullScreen: .cover
      case .popover: .popover
      case .overCurrentContext: .overlay
      default: .sheet
      }
    }

    private func resolveContext(
      nodes: [UIKitSemanticNode]
    ) -> UIKitFactContext {
      var annotationContext = AnnotationContext()
      var policy = ViewCapturePolicy.standard
      var seen: Set<ObjectIdentifier> = []
      for node in nodes where seen.insert(ObjectIdentifier(node)).inserted {
        if !node.declarations.isEmpty {
          annotationContext =
            (try? annotationContext.addingScope(
              id: node.scopeID,
              declarations: node.declarations
            )) ?? annotationContext
        }
        policy = policy.restricting(
          privacy: node.policy.privacy,
          replay: node.policy.replay
        )
      }
      return UIKitFactContext(
        annotations: annotationContext.snapshot,
        page: nil,
        policy: policy
      )
    }

    private func updateCause(
      for candidate: Candidate,
      fact: UIKitPageFact,
      sceneTransitioned: Bool
    ) -> PageCause {
      guard let previous = candidate.state.lifetime.latest else {
        return candidate.state.declaration.cause
      }
      if sceneTransitioned {
        return fact.path.exposure == .retained ? .background : .foreground
      }
      if previous.path.exposure == .foreground,
        fact.path.exposure == .visible || fact.path.exposure == .occluded
      {
        return .present
      }
      if previous.path.exposure == .visible
        || previous.path.exposure == .occluded,
        fact.path.exposure == .foreground
      {
        return .dismiss
      }
      switch fact.path.relation {
      case .tab, .split: return .selection
      case .sheet, .popover, .overlay, .cover: return .present
      case .root, .push: return .navigate
      }
    }

    private func initialCause(for candidate: Candidate) -> PageCause {
      guard candidate.state.declaration.cause == .navigate else {
        return candidate.state.declaration.cause
      }
      if candidate.depth == 0 { return .initial }
      switch candidate.relation {
      case .tab, .split: return .selection
      case .sheet, .popover, .overlay, .cover: return .present
      case .root, .push: return .navigate
      }
    }

    private func observeSceneIfNeeded() {
      guard let scene = window?.windowScene, scene !== observedScene else {
        return
      }
      if let observedScene {
        NotificationCenter.default.removeObserver(
          self,
          name: nil,
          object: observedScene
        )
      }
      observedScene = scene
      let names: [Notification.Name] = [
        UIScene.didActivateNotification,
        UIScene.willDeactivateNotification,
        UIScene.willEnterForegroundNotification,
        UIScene.didEnterBackgroundNotification,
        UIScene.didDisconnectNotification,
      ]
      for name in names {
        NotificationCenter.default.addObserver(
          self,
          selector: #selector(sceneLifecycleChanged),
          name: name,
          object: scene
        )
      }
    }

    @objc private func sceneLifecycleChanged(_: Notification) {
      requestReconciliation()
    }

    private func emitSceneLifecycleIfNeeded() {
      guard let state = window?.windowScene?.activationState,
        state != lastSceneState,
        let rootFact = active.values.compactMap(\.lifetime.latest)
          .min(by: { $0.path.instanceIDs.count < $1.path.instanceIDs.count })
      else {
        return
      }
      lastSceneState = state
      let rawName: String =
        switch state {
        case .foregroundActive: "app.scene.active"
        case .foregroundInactive: "app.scene.inactive"
        case .background: "app.scene.background"
        case .unattached: "app.scene.unattached"
        @unknown default: "app.scene.unknown"
        }
      guard let name = try? SemanticName(rawName) else { return }
      UIKitFactEmitter.lifecycleEvent(
        name: name,
        context: UIKitFactContext(
          annotations: rootFact.annotations,
          page: rootFact.path,
          policy: rootFact.policy
        )
      )
    }

    func registerImpression(_ observer: UIKitImpressionObserver) {
      impressionObservers[ObjectIdentifier(observer)] = WeakImpressionObserver(
        observer
      )
    }

    func unregisterImpression(_ observer: UIKitImpressionObserver) {
      impressionObservers.removeValue(forKey: ObjectIdentifier(observer))
    }

    func claimNativeActivation(_ token: UIKitNativeActivationToken) -> Bool {
      guard nativeActivations.insert(token).inserted else { return false }
      nativeActivationOrder.append(token)
      if nativeActivationOrder.count > Self.nativeActivationLimit {
        nativeActivations.remove(nativeActivationOrder.removeFirst())
      }
      return true
    }

    private func evaluateImpressions() {
      var stale: [ObjectIdentifier] = []
      for (identifier, reference) in impressionObservers {
        guard let observer = reference.value else {
          stale.append(identifier)
          continue
        }
        observer.evaluateVisibility()
      }
      for identifier in stale {
        impressionObservers.removeValue(forKey: identifier)
      }
    }

    private func finishAllPages() {
      for state in active.values.sorted(by: { $0.depth > $1.depth }) {
        graph.remove(state.instanceID)
        state.active = false
        state.lifetime.finish()
        state.contextNodes = []
      }
      active.removeAll()
    }

    deinit {
      NotificationCenter.default.removeObserver(self)
    }
  }

  @MainActor
  final class UIKitPageProbeView: UIView {
    weak var controller: UIViewController?

    init(controller: UIViewController) {
      self.controller = controller
      super.init(frame: .zero)
      isHidden = true
      isUserInteractionEnabled = false
      isAccessibilityElement = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
      fatalError("init(coder:) is unavailable")
    }

    override func willMove(toWindow newWindow: UIWindow?) {
      let previousWindow = window
      super.willMove(toWindow: newWindow)
      if let previousWindow, previousWindow !== newWindow {
        surfaceCoordinator(for: previousWindow)?.requestReconciliation()
      }
    }

    override func didMoveToWindow() {
      super.didMoveToWindow()
      if let window {
        surfaceCoordinator(for: window)?.requestReconciliation()
      }
    }

    override func layoutSubviews() {
      super.layoutSubviews()
      if let window {
        surfaceCoordinator(for: window)?.requestReconciliation()
      }
    }
  }

  @MainActor
  func installUIKitPageProbe(on controller: UIViewController) {
    guard let node = semanticNode(for: controller) else { return }
    if node.pageProbe?.superview !== controller.view {
      node.pageProbe?.removeFromSuperview()
      let probe = UIKitPageProbeView(controller: controller)
      node.pageProbe = probe
      controller.view.addSubview(probe)
    }
  }
#endif
