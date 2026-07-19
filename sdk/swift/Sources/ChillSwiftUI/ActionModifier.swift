import ChillCore
import SwiftUI

private struct ChillActionModifier: ViewModifier {
  @Environment(\.chillAnnotationContext) private var annotations
  @Environment(\.chillCapturePolicy) private var policy
  @Environment(\.chillPageAncestry) private var page
  @Environment(\.isEnabled) private var isEnabled
  @State private var elementInstanceID = newElementInstanceID()
  @State private var fallbackSurfaceID = newSurfaceID()

  let name: SemanticName?
  let role: SemanticName?
  let activation: ActionActivation

  @ViewBuilder
  func body(content: Content) -> some View {
    if activation == .submit {
      content.onSubmit {
        emit(input: .unknown)
      }
    } else {
      content.simultaneousGesture(
        TapGesture().onEnded {
          emit(input: .unknown)
        }
      )
    }
  }

  private func emit(input: InputKind) {
    guard isEnabled, let name, let role else { return }
    ChillUIInstrumentation.action(
      name: name,
      role: role,
      activation: activation,
      input: input,
      surfaceID: page?.path.surfaceID ?? fallbackSurfaceID,
      elementInstanceID: elementInstanceID,
      context: SwiftUIFactContext(
        annotations: annotations.snapshot,
        page: page?.path,
        policy: policy
      )
    )
  }
}

extension View {
  /// Declares a semantic tap or submit command without capturing the label.
  ///
  /// Generic SwiftUI gestures do not expose a trustworthy input origin, so
  /// this modifier records `unknown`. Use `ChillActionButton` for a native
  /// Button boundary that includes every platform activation mechanism.
  public func action(
    _ name: String,
    role: String = "button",
    activation: ActionActivation = .primary
  ) -> some View {
    modifier(
      ChillActionModifier(
        name: try? SemanticName(name),
        role: try? SemanticName(role),
        activation: activation
      )
    )
  }
}

/// A native SwiftUI Button with an exact semantic action boundary.
///
/// SwiftUI invokes the wrapped action for touch, pointer, keyboard, remote,
/// voice, and accessibility activation. Chill records the command once at
/// that committed boundary and does not capture the visible label.
public struct ChillActionButton<Label: View>: View {
  @Environment(\.chillAnnotationContext) private var annotations
  @Environment(\.chillCapturePolicy) private var policy
  @Environment(\.chillPageAncestry) private var page
  @State private var elementInstanceID = newElementInstanceID()
  @State private var fallbackSurfaceID = newSurfaceID()

  private let name: SemanticName?
  private let semanticRole: SemanticName?
  private let activation: ActionActivation
  private let buttonRole: ButtonRole?
  private let operation: @MainActor () -> Void
  private let label: () -> Label

  public init(
    _ name: String,
    role: String = "button",
    activation: ActionActivation = .primary,
    buttonRole: ButtonRole? = nil,
    operation: @escaping @MainActor () -> Void,
    @ViewBuilder label: @escaping () -> Label
  ) {
    self.name = try? SemanticName(name)
    semanticRole = try? SemanticName(role)
    self.activation = activation
    self.buttonRole = buttonRole
    self.operation = operation
    self.label = label
  }

  public var body: some View {
    Button(role: buttonRole) {
      _ChillInstrumentation.withActionTrace(name: name) {
        if let name, let semanticRole {
          ChillUIInstrumentation.action(
            name: name,
            role: semanticRole,
            activation: activation,
            input: .unknown,
            surfaceID: page?.path.surfaceID ?? fallbackSurfaceID,
            elementInstanceID: elementInstanceID,
            context: SwiftUIFactContext(
              annotations: annotations.snapshot,
              page: page?.path,
              policy: policy
            )
          )
        }
        _ChillInstrumentation.withUIContext(
          annotations: annotations.snapshot,
          page: page?.path,
          operation: operation
        )
      }
    } label: {
      label()
    }
  }
}

extension ChillActionButton where Label == Text {
  public init<S: StringProtocol>(
    _ title: S,
    action name: String,
    role: String = "button",
    activation: ActionActivation = .primary,
    buttonRole: ButtonRole? = nil,
    operation: @escaping @MainActor () -> Void
  ) {
    self.init(
      name,
      role: role,
      activation: activation,
      buttonRole: buttonRole,
      operation: operation
    ) {
      Text(title)
    }
  }
}
