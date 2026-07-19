#if os(iOS)
  import ChillCore
  import UIKit

  struct UIKitNativeActivationToken: Hashable {
    let eventIdentity: ObjectIdentifier
    let timestampBits: UInt64
  }

  @MainActor
  final class UIKitControlActionObserver: NSObject {
    weak var control: UIControl?
    weak var node: UIKitSemanticNode?
    private var installedEvents: UIControl.Event = []

    init(control: UIControl, node: UIKitSemanticNode) {
      self.control = control
      self.node = node
      super.init()
      refresh()
    }

    func refresh() {
      guard let control else { return }
      if !installedEvents.isEmpty {
        control.removeTarget(
          self,
          action: #selector(activated(_:event:)),
          for: installedEvents
        )
      }
      installedEvents = node?.action?.controlEvents ?? []
      if !installedEvents.isEmpty {
        control.addTarget(
          self,
          action: #selector(activated(_:event:)),
          for: installedEvents
        )
      }
    }

    @objc private func activated(_ sender: UIControl, event: UIEvent?) {
      receiveActivation(sender, event: event)
    }

    func receiveActivation(_ sender: UIControl, event: UIEvent?) {
      guard sender.isEnabled, let declaration = node?.action else { return }
      let (context, surfaceID, node) = resolveUIKitFactContext(for: sender)
      if let event,
        let coordinator = sender.window.flatMap({
          surfaceCoordinator(for: $0)
        }),
        !coordinator.claimNativeActivation(
          UIKitNativeActivationToken(
            eventIdentity: ObjectIdentifier(event),
            timestampBits: event.timestamp.bitPattern
          )
        )
      {
        UIKitFactEmitter.duplicateActionObservation(context: context)
        return
      }
      UIKitFactEmitter.action(
        name: declaration.name,
        role: declaration.role,
        activation: declaration.activation,
        input: inputKind(for: event),
        surfaceID: surfaceID,
        elementInstanceID: node.elementInstanceID,
        context: context
      )
    }

    private func inputKind(for event: UIEvent?) -> InputKind {
      guard let event else { return .unknown }
      switch event.type {
      case .touches:
        if event.allTouches?.contains(where: {
          $0.type == .indirectPointer || $0.type == .indirect
        }) == true {
          return .pointer
        }
        return .touch
      case .presses:
        if let presses = event as? UIPressesEvent,
          presses.allPresses.contains(where: { $0.key != nil })
        {
          return .keyboard
        }
        return .remote
      case .remoteControl: return .remote
      case .scroll, .hover, .transform: return .pointer
      case .motion: return .system
      @unknown default: return .unknown
      }
    }

  }

  @MainActor
  final class UIKitGestureActionObserver: NSObject {
    weak var gesture: UIGestureRecognizer?
    weak var node: UIKitSemanticNode?
    private var installed = false

    init(gesture: UIGestureRecognizer, node: UIKitSemanticNode) {
      self.gesture = gesture
      self.node = node
      super.init()
      refresh()
    }

    func refresh() {
      guard let gesture else { return }
      if installed {
        gesture.removeTarget(self, action: #selector(recognized(_:)))
      }
      installed = node?.action != nil
      if installed {
        gesture.addTarget(self, action: #selector(recognized(_:)))
      }
    }

    @objc private func recognized(_ sender: UIGestureRecognizer) {
      guard sender.state == .ended, let declaration = node?.action else {
        return
      }
      let (context, surfaceID, node) = resolveUIKitFactContext(for: sender)
      guard !hasCloserDeclaredControl(sender) else {
        UIKitFactEmitter.duplicateActionObservation(context: context)
        return
      }
      UIKitFactEmitter.action(
        name: declaration.name,
        role: declaration.role,
        activation: declaration.activation,
        input: .unknown,
        surfaceID: surfaceID,
        elementInstanceID: node.elementInstanceID,
        context: context
      )
    }

    private func hasCloserDeclaredControl(
      _ gesture: UIGestureRecognizer
    ) -> Bool {
      guard let container = gesture.view else { return false }
      var view: UIView? = container.hitTest(
        gesture.location(in: container),
        with: nil
      )
      while let current = view {
        if let control = current as? UIControl,
          semanticNode(for: control, create: false)?.action != nil
        {
          return true
        }
        if current === container { break }
        view = current.superview
      }
      return false
    }

  }

  @MainActor
  func installUIKitControlActionObserver(
    on control: UIControl,
    node: UIKitSemanticNode
  ) {
    if let observer = node.controlObserver {
      observer.refresh()
    } else {
      node.controlObserver = UIKitControlActionObserver(
        control: control,
        node: node
      )
    }
  }

  @MainActor
  func installUIKitGestureActionObserver(
    on gesture: UIGestureRecognizer,
    node: UIKitSemanticNode
  ) {
    if let observer = node.gestureObserver {
      observer.refresh()
    } else {
      node.gestureObserver = UIKitGestureActionObserver(
        gesture: gesture,
        node: node
      )
    }
  }
#endif
