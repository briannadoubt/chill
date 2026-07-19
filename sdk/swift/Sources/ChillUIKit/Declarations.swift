#if os(iOS)
  import ChillCore
  import ObjectiveC
  import UIKit

  /// UIKit objects adopt this marker to receive Chill's declarative semantic
  /// configuration API. Calling these methods declares meaning once during
  /// normal view setup; it does not emit an event.
  @MainActor
  public protocol ChillUIKitAnnotatable: AnyObject {}

  extension UIView: ChillUIKitAnnotatable {}
  extension UIViewController: ChillUIKitAnnotatable {}
  extension UIGestureRecognizer: ChillUIKitAnnotatable {}

  @MainActor
  final class UIKitSemanticNode {
    var scopeID = newUIKitAnnotationScopeID()
    var elementInstanceID = newUIKitElementInstanceID()
    var fallbackSurfaceID = newUIKitSurfaceID()
    private(set) var annotationOrder: [AnnotationName] = []
    private(set) var annotations: [AnnotationName: AnnotationDeclaration] = [:]
    private(set) var policy = ViewCapturePolicy.standard
    var pageState: UIKitPageState?
    var pageProbe: UIKitPageProbeView?
    var action: UIKitActionDeclaration?
    var controlObserver: UIKitControlActionObserver?
    var gestureObserver: UIKitGestureActionObserver?
    var impression: UIKitImpressionDeclaration?
    var impressionObserver: UIKitImpressionObserver?
    var impressionProbe: UIKitImpressionProbeView?

    var declarations: [AnnotationDeclaration] {
      annotationOrder.compactMap { annotations[$0] }
    }

    func declare(_ declaration: AnnotationDeclaration) {
      if annotations[declaration.name] == nil {
        annotationOrder.append(declaration.name)
      }
      annotations[declaration.name] = declaration
    }

    func restrict(
      privacy: ViewPrivacyDisposition? = nil,
      replay: ReplayDisposition? = nil
    ) {
      policy = policy.restricting(privacy: privacy, replay: replay)
    }

    func resetForReuse() {
      scopeID = newUIKitAnnotationScopeID()
      elementInstanceID = newUIKitElementInstanceID()
      annotationOrder.removeAll(keepingCapacity: true)
      annotations.removeAll(keepingCapacity: true)
      impressionObserver?.resetForReuse()
    }
  }

  @MainActor
  final class UIKitPageState {
    private final class WeakSemanticNode {
      weak var value: UIKitSemanticNode?

      init(_ value: UIKitSemanticNode) {
        self.value = value
      }
    }

    var declaration: UIKitPageDeclaration
    var instanceID = newUIKitPageInstanceID()
    var lifetime = UIKitPageLifetime()
    var active = false
    var depth = 0
    private var contextNodeReferences: [WeakSemanticNode] = []

    var contextNodes: [UIKitSemanticNode] {
      get { contextNodeReferences.compactMap(\.value) }
      set { contextNodeReferences = newValue.map(WeakSemanticNode.init) }
    }

    init(declaration: UIKitPageDeclaration) {
      self.declaration = declaration
    }

    func prepareForInsertion() {
      guard lifetime.hasEnded else { return }
      instanceID = newUIKitPageInstanceID()
      lifetime = UIKitPageLifetime()
    }
  }

  struct UIKitPageDeclaration: Equatable {
    let segment: PageSegment
    let relation: PageRelation?
    let cause: PageCause
  }

  struct UIKitActionDeclaration: Equatable {
    let name: SemanticName
    let role: SemanticName
    let activation: ActionActivation
    let controlEvents: UIControl.Event

    static func controlEvents(
      for activation: ActionActivation
    ) -> UIControl.Event {
      switch activation {
      case .submit: .editingDidEndOnExit
      case .toggle, .selection, .adjust: .valueChanged
      case .primary, .gesture, .system: .primaryActionTriggered
      }
    }
  }

  struct UIKitImpressionDeclaration: Equatable {
    let name: SemanticName
    let role: SemanticName
    let visibilityThreshold: Double
    let minimumVisibleDuration: Duration
    let repeats: Bool
  }

  @MainActor
  private enum UIKitAssociationKeys {
    nonisolated(unsafe) static var semanticNode: UInt8 = 0
    nonisolated(unsafe) static var surfaceCoordinator: UInt8 = 0
  }

  @MainActor
  func semanticNode(
    for object: AnyObject,
    create: Bool = true
  ) -> UIKitSemanticNode? {
    if let existing = objc_getAssociatedObject(
      object,
      &UIKitAssociationKeys.semanticNode
    ) as? UIKitSemanticNode {
      return existing
    }
    guard create else { return nil }
    let node = UIKitSemanticNode()
    objc_setAssociatedObject(
      object,
      &UIKitAssociationKeys.semanticNode,
      node,
      .OBJC_ASSOCIATION_RETAIN_NONATOMIC
    )
    return node
  }

  /// Package SPI used by the separately linked replay adapter. It exposes only
  /// declared semantic identifiers and monotone policy, never UI content.
  package struct UIKitReplayMetadata: Sendable {
    package let semanticID: String?
    package let semanticRole: String?
    package let policy: ViewCapturePolicy
  }

  @MainActor
  package func replayMetadata(for object: AnyObject) -> UIKitReplayMetadata {
    guard let node = semanticNode(for: object, create: false) else {
      return UIKitReplayMetadata(
        semanticID: nil,
        semanticRole: nil,
        policy: .standard
      )
    }
    return UIKitReplayMetadata(
      semanticID: node.action?.name.rawValue,
      semanticRole: node.action?.role.rawValue,
      policy: node.policy
    )
  }

  @MainActor
  func surfaceCoordinator(
    for window: UIWindow,
    create: Bool = true
  ) -> UIKitSurfaceCoordinator? {
    if let existing = objc_getAssociatedObject(
      window,
      &UIKitAssociationKeys.surfaceCoordinator
    ) as? UIKitSurfaceCoordinator {
      return existing
    }
    guard create else { return nil }
    let coordinator = UIKitSurfaceCoordinator(window: window)
    objc_setAssociatedObject(
      window,
      &UIKitAssociationKeys.surfaceCoordinator,
      coordinator,
      .OBJC_ASSOCIATION_RETAIN_NONATOMIC
    )
    return coordinator
  }

  extension ChillUIKitAnnotatable {
    /// Declares one bounded semantic annotation on this object. Ancestor
    /// objects are resolved first, so an outer declaration wins a collision.
    @discardableResult
    public func annotation<Value: AnnotationValueConvertible>(
      _ name: String,
      value: Value
    ) -> Self {
      guard let declaration = try? AnnotationDeclaration(name, value: value)
      else {
        return self
      }
      semanticNode(for: self)?.declare(declaration)
      requestUIKitReconciliation(for: self)
      return self
    }

    @discardableResult
    public func annotation<Value: AnnotationValueConvertible>(
      _ key: AnnotationKey<Value>,
      value: Value
    ) -> Self {
      guard let declaration = try? AnnotationDeclaration(key, value: value)
      else {
        return self
      }
      semanticNode(for: self)?.declare(declaration)
      requestUIKitReconciliation(for: self)
      return self
    }

    /// Declares a deterministic homogeneous map as one logical object scope.
    @discardableResult
    public func annotation<Value: AnnotationValueConvertible>(
      _ values: [String: Value]
    ) -> Self {
      for name in values.keys.sorted() {
        guard let value = values[name],
          let declaration = try? AnnotationDeclaration(name, value: value)
        else {
          continue
        }
        semanticNode(for: self)?.declare(declaration)
      }
      requestUIKitReconciliation(for: self)
      return self
    }

    /// Restricts analytics for this object subtree. Restrictions are monotone.
    @discardableResult
    public func privacy(_ disposition: ViewPrivacyDisposition) -> Self {
      semanticNode(for: self)?.restrict(privacy: disposition)
      requestUIKitReconciliation(for: self)
      return self
    }

    /// Restricts structural replay for this object subtree.
    @discardableResult
    public func replay(_ disposition: ReplayDisposition) -> Self {
      semanticNode(for: self)?.restrict(replay: disposition)
      requestUIKitReconciliation(for: self)
      return self
    }
  }

  extension UIViewController {
    /// Declares one stable page segment for this logical controller lifetime.
    /// Native navigation, tab, split, and presentation containers infer the
    /// relation when `relation` is omitted.
    @discardableResult
    public func page(
      _ segment: String,
      relation: PageRelation? = nil,
      cause: PageCause = .navigate
    ) -> Self {
      guard let segment = try? PageSegment(segment),
        let node = semanticNode(for: self)
      else {
        return self
      }
      let declaration = UIKitPageDeclaration(
        segment: segment,
        relation: relation,
        cause: cause
      )
      if let state = node.pageState {
        state.declaration = declaration
      } else {
        node.pageState = UIKitPageState(declaration: declaration)
      }
      installUIKitPageProbe(on: self)
      requestUIKitReconciliation(for: self)
      return self
    }
  }

  extension UIControl {
    /// Declares the committed native control command. The observer is attached
    /// once and emits automatically whenever UIKit dispatches `events`.
    @discardableResult
    public func action(
      _ name: String,
      role: String = "button",
      activation: ActionActivation = .primary,
      events: UIControl.Event? = nil
    ) -> Self {
      guard let name = try? SemanticName(name),
        let role = try? SemanticName(role),
        let node = semanticNode(for: self)
      else {
        return self
      }
      node.action = UIKitActionDeclaration(
        name: name,
        role: role,
        activation: activation,
        controlEvents: events
          ?? UIKitActionDeclaration.controlEvents(for: activation)
      )
      installUIKitControlActionObserver(on: self, node: node)
      return self
    }
  }

  extension UIGestureRecognizer {
    /// Declares an action at the recognizer's successful terminal boundary.
    @discardableResult
    public func action(
      _ name: String,
      role: String = "gesture",
      activation: ActionActivation = .gesture
    ) -> Self {
      guard let name = try? SemanticName(name),
        let role = try? SemanticName(role),
        let node = semanticNode(for: self)
      else {
        return self
      }
      node.action = UIKitActionDeclaration(
        name: name,
        role: role,
        activation: activation,
        controlEvents: []
      )
      installUIKitGestureActionObserver(on: self, node: node)
      return self
    }
  }

  extension UIView {
    /// Declares a geometry-based impression with cancellable viewport dwell.
    @discardableResult
    public func impression(
      _ name: String,
      role: String = "content",
      visibilityThreshold: Double = 0.5,
      minimumVisibleDuration: Duration = .milliseconds(500),
      repeats: Bool = false
    ) -> Self {
      guard let name = try? SemanticName(name),
        let role = try? SemanticName(role),
        let node = semanticNode(for: self)
      else {
        return self
      }
      node.impression = UIKitImpressionDeclaration(
        name: name,
        role: role,
        visibilityThreshold: visibilityThreshold.isFinite
          ? min(max(visibilityThreshold, 0), 1) : 1,
        minimumVisibleDuration: max(minimumVisibleDuration, .zero),
        repeats: repeats
      )
      installUIKitImpressionObserver(on: self, node: node)
      return self
    }
  }

  @MainActor
  func requestUIKitReconciliation(for object: AnyObject) {
    let window: UIWindow?
    switch object {
    case let view as UIView:
      window = view.window
    case let controller as UIViewController:
      window = controller.viewIfLoaded?.window
    case let gesture as UIGestureRecognizer:
      window = gesture.view?.window
    default:
      window = nil
    }
    guard let window else { return }
    surfaceCoordinator(for: window)?.requestReconciliation()
  }

  @MainActor
  func resolveUIKitFactContext(
    for object: AnyObject
  ) -> (UIKitFactContext, SurfaceID, UIKitSemanticNode) {
    let ownNode = semanticNode(for: object)!
    let view: UIView? =
      switch object {
      case let value as UIView: value
      case let value as UIGestureRecognizer: value.view
      case let value as UIViewController: value.viewIfLoaded
      default: nil
      }
    let controller =
      view.flatMap(owningViewController)
      ?? (object as? UIViewController)
    let pageState = controller.flatMap {
      nearestPageState(from: $0)
    }
    var orderedNodes =
      pageState?.contextNodes
      ?? controller.map(basicControllerNodes) ?? []
    if let view {
      let viewNodes = viewHierarchy(from: view).compactMap {
        semanticNode(for: $0, create: false)
      }
      orderedNodes.append(contentsOf: viewNodes)
    }
    if object is UIGestureRecognizer {
      orderedNodes.append(ownNode)
    }

    var seen: Set<ObjectIdentifier> = []
    var annotations = AnnotationContext()
    var policy = ViewCapturePolicy.standard
    for node in orderedNodes where seen.insert(ObjectIdentifier(node)).inserted {
      if !node.declarations.isEmpty {
        annotations =
          (try? annotations.addingScope(
            id: node.scopeID,
            declarations: node.declarations
          )) ?? annotations
      }
      policy = policy.restricting(
        privacy: node.policy.privacy,
        replay: node.policy.replay
      )
    }
    let page = pageState?.lifetime.latest?.path
    let surfaceID =
      page?.surfaceID
      ?? view?.window.flatMap {
        surfaceCoordinator(for: $0)?.surfaceID
      }
      ?? ownNode.fallbackSurfaceID
    return (
      UIKitFactContext(
        annotations: annotations.snapshot,
        page: page,
        policy: policy
      ),
      surfaceID,
      ownNode
    )
  }

  @MainActor
  private func owningViewController(_ view: UIView) -> UIViewController? {
    var responder: UIResponder? = view
    while let current = responder {
      if let controller = current as? UIViewController { return controller }
      responder = current.next
    }
    return nil
  }

  @MainActor
  private func nearestPageState(
    from controller: UIViewController
  ) -> UIKitPageState? {
    if let state = semanticNode(for: controller, create: false)?.pageState,
      state.active
    {
      return state
    }
    var cursor = controller.parent
    while let current = cursor {
      if let state = semanticNode(for: current, create: false)?.pageState,
        state.active
      {
        return state
      }
      cursor = current.parent
    }
    return nil
  }

  @MainActor
  private func basicControllerNodes(
    _ controller: UIViewController
  ) -> [UIKitSemanticNode] {
    var controllers: [UIViewController] = []
    var cursor: UIViewController? = controller
    while let current = cursor {
      controllers.append(current)
      cursor = current.parent ?? current.presentingViewController
    }
    return controllers.reversed().compactMap {
      semanticNode(for: $0, create: false)
    }
  }

  @MainActor
  private func viewHierarchy(from view: UIView) -> [UIView] {
    var views: [UIView] = []
    var cursor: UIView? = view
    while let current = cursor {
      views.append(current)
      cursor = current.superview
    }
    return views.reversed()
  }

  @MainActor
  func resetUIKitReusableState(of view: UIView) {
    semanticNode(for: view, create: false)?.resetForReuse()
    for gesture in view.gestureRecognizers ?? [] {
      semanticNode(for: gesture, create: false)?.resetForReuse()
    }
    for child in view.subviews {
      resetUIKitReusableState(of: child)
    }
  }
#endif
