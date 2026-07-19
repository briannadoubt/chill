import ChillCore
import SwiftUI

/// A native Toggle whose binding commit is one semantic toggle action.
/// The Boolean value and visible label are never captured.
public struct ChillToggle<Label: View>: View {
  @Environment(\.chillAnnotationContext) private var annotations
  @Environment(\.chillCapturePolicy) private var policy
  @Environment(\.chillPageAncestry) private var page
  @State private var elementInstanceID = newElementInstanceID()
  @State private var fallbackSurfaceID = newSurfaceID()

  @Binding private var isOn: Bool
  private let name: SemanticName?
  private let role: SemanticName?
  private let label: () -> Label

  public init(
    _ name: String,
    role: String = "switch",
    isOn: Binding<Bool>,
    @ViewBuilder label: @escaping () -> Label
  ) {
    self.name = try? SemanticName(name)
    self.role = try? SemanticName(role)
    _isOn = isOn
    self.label = label
  }

  public var body: some View {
    Toggle(isOn: instrumentedBinding, label: label)
  }

  private var instrumentedBinding: Binding<Bool> {
    Binding(
      get: { isOn },
      set: { newValue in
        guard newValue != isOn else { return }
        _ChillInstrumentation.withActionTrace(name: name) {
          emit()
          isOn = newValue
        }
      }
    )
  }

  private func emit() {
    guard let name, let role else { return }
    ChillUIInstrumentation.action(
      name: name,
      role: role,
      activation: .toggle,
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
}

extension ChillToggle where Label == Text {
  public init<S: StringProtocol>(
    _ title: S,
    action name: String,
    role: String = "switch",
    isOn: Binding<Bool>
  ) {
    self.init(name, role: role, isOn: isOn) {
      Text(title)
    }
  }
}

/// A native Picker whose selection binding commit is one semantic selection.
/// The selected value and labels are never captured.
public struct ChillPicker<
  Label: View,
  SelectionValue: Hashable,
  Content: View
>: View {
  @Environment(\.chillAnnotationContext) private var annotations
  @Environment(\.chillCapturePolicy) private var policy
  @Environment(\.chillPageAncestry) private var page
  @State private var elementInstanceID = newElementInstanceID()
  @State private var fallbackSurfaceID = newSurfaceID()

  @Binding private var selection: SelectionValue
  private let name: SemanticName?
  private let role: SemanticName?
  private let content: () -> Content
  private let label: () -> Label

  public init(
    _ name: String,
    role: String = "picker",
    selection: Binding<SelectionValue>,
    @ViewBuilder content: @escaping () -> Content,
    @ViewBuilder label: @escaping () -> Label
  ) {
    self.name = try? SemanticName(name)
    self.role = try? SemanticName(role)
    _selection = selection
    self.content = content
    self.label = label
  }

  public var body: some View {
    Picker(
      selection: instrumentedBinding,
      content: content,
      label: label
    )
  }

  private var instrumentedBinding: Binding<SelectionValue> {
    Binding(
      get: { selection },
      set: { newValue in
        guard newValue != selection else { return }
        _ChillInstrumentation.withActionTrace(name: name) {
          emit()
          selection = newValue
        }
      }
    )
  }

  private func emit() {
    guard let name, let role else { return }
    ChillUIInstrumentation.action(
      name: name,
      role: role,
      activation: .selection,
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
}
