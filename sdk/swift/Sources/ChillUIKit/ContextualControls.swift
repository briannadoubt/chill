#if os(iOS)
  import ChillCore
  import UIKit

  @MainActor
  func withUIKitControlContext(
    control: UIControl,
    operation: () -> Void
  ) {
    let (context, _, _) = resolveUIKitFactContext(for: control)
    let name = semanticNode(for: control, create: false)?.action?.name
    _ChillInstrumentation.withActionTrace(name: name) {
      _ChillInstrumentation.withUIContext(
        annotations: context.annotations,
        page: context.page,
        operation: operation
      )
    }
  }

  /// A native button that automatically carries Chill UI context through
  /// target-action dispatch. Configure its semantic action once with
  /// `.action(...)`; application callbacks remain ordinary callbacks.
  open class ChillButton: UIButton {
    public override init(frame: CGRect) {
      super.init(frame: frame)
      isAccessibilityElement = true
    }

    public required init?(coder: NSCoder) {
      super.init(coder: coder)
      isAccessibilityElement = true
    }

    open override func sendActions(for controlEvents: UIControl.Event) {
      withUIKitControlContext(control: self) {
        super.sendActions(for: controlEvents)
      }
    }

    open override func sendAction(
      _ action: Selector,
      to target: Any?,
      for event: UIEvent?
    ) {
      withUIKitControlContext(control: self) {
        super.sendAction(action, to: target, for: event)
      }
    }
  }

  /// A native switch with automatic macro context propagation.
  open class ChillSwitch: UISwitch {
    open override func sendActions(for controlEvents: UIControl.Event) {
      withUIKitControlContext(control: self) {
        super.sendActions(for: controlEvents)
      }
    }

    open override func sendAction(
      _ action: Selector,
      to target: Any?,
      for event: UIEvent?
    ) {
      withUIKitControlContext(control: self) {
        super.sendAction(action, to: target, for: event)
      }
    }
  }

  /// A native segmented control with automatic macro context propagation.
  open class ChillSegmentedControl: UISegmentedControl {
    open override func sendActions(for controlEvents: UIControl.Event) {
      withUIKitControlContext(control: self) {
        super.sendActions(for: controlEvents)
      }
    }

    open override func sendAction(
      _ action: Selector,
      to target: Any?,
      for event: UIEvent?
    ) {
      withUIKitControlContext(control: self) {
        super.sendAction(action, to: target, for: event)
      }
    }
  }

  /// A native slider with automatic macro context propagation.
  open class ChillSlider: UISlider {
    open override func sendActions(for controlEvents: UIControl.Event) {
      withUIKitControlContext(control: self) {
        super.sendActions(for: controlEvents)
      }
    }

    open override func sendAction(
      _ action: Selector,
      to target: Any?,
      for event: UIEvent?
    ) {
      withUIKitControlContext(control: self) {
        super.sendAction(action, to: target, for: event)
      }
    }
  }

  /// A native stepper with automatic macro context propagation.
  open class ChillStepper: UIStepper {
    open override func sendActions(for controlEvents: UIControl.Event) {
      withUIKitControlContext(control: self) {
        super.sendActions(for: controlEvents)
      }
    }

    open override func sendAction(
      _ action: Selector,
      to target: Any?,
      for event: UIEvent?
    ) {
      withUIKitControlContext(control: self) {
        super.sendAction(action, to: target, for: event)
      }
    }
  }

  /// A native text field that propagates semantic context but never reads or
  /// serializes entered text.
  open class ChillTextField: UITextField {
    open override func sendActions(for controlEvents: UIControl.Event) {
      withUIKitControlContext(control: self) {
        super.sendActions(for: controlEvents)
      }
    }

    open override func sendAction(
      _ action: Selector,
      to target: Any?,
      for event: UIEvent?
    ) {
      withUIKitControlContext(control: self) {
        super.sendAction(action, to: target, for: event)
      }
    }
  }
#endif
