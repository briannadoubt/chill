import ChillCore
import SwiftUI

private struct ChillObservedEventModifier<Value: Equatable>: ViewModifier {
  @Environment(\.chillAnnotationContext) private var annotations
  @Environment(\.chillCapturePolicy) private var policy
  @Environment(\.chillPageAncestry) private var page
  @State private var emittedInitial = false

  let name: SemanticName?
  let value: Value
  let eventClass: EventClass
  let severity: EventSeverity
  let emitInitial: Bool

  func body(content: Content) -> some View {
    content
      .onAppear {
        guard emitInitial, !emittedInitial else { return }
        emittedInitial = true
        emit()
      }
      .onChange(of: value) { _, _ in
        emit()
      }
  }

  private func emit() {
    guard let name else { return }
    ChillUIInstrumentation.observedEvent(
      name: name,
      eventClass: eventClass,
      severity: severity,
      context: SwiftUIFactContext(
        annotations: annotations.snapshot,
        page: page?.path,
        policy: policy
      )
    )
  }
}

extension View {
  /// Emits an observed event when an Equatable state projection changes.
  /// The state value is a trigger only and is never placed in the record.
  public func event<Value: Equatable>(
    _ name: String,
    when value: Value,
    eventClass: EventClass = .domain,
    severity: EventSeverity = .info,
    emitInitial: Bool = false
  ) -> some View {
    modifier(
      ChillObservedEventModifier(
        name: try? SemanticName(name),
        value: value,
        eventClass: eventClass,
        severity: severity,
        emitInitial: emitInitial
      )
    )
  }
}
