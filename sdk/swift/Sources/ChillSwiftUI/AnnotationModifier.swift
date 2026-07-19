import ChillCore
import SwiftUI

private struct ChillAnnotationModifier: ViewModifier {
  @Environment(\.chillAnnotationContext) private var inherited
  @State private var scopeID = newAnnotationScopeID()

  let declarations: [AnnotationDeclaration]

  func body(content: Content) -> some View {
    let resolved =
      (try? inherited.addingScope(
        id: scopeID,
        declarations: declarations
      )) ?? inherited
    content.environment(\.chillAnnotationContext, resolved)
  }
}

extension View {
  /// Adds one bounded semantic value to this logical view scope.
  ///
  /// SwiftUI modifiers wrap inside-out, so a later annotation modifier is
  /// structurally outer and wins a collision with an earlier modifier.
  public func annotation<Value: AnnotationValueConvertible>(
    _ name: String,
    value: Value
  ) -> some View {
    let declarations =
      (try? AnnotationDeclaration(name, value: value)).map {
        [$0]
      } ?? []
    return modifier(ChillAnnotationModifier(declarations: declarations))
  }

  public func annotation<Value: AnnotationValueConvertible>(
    _ key: AnnotationKey<Value>,
    value: Value
  ) -> some View {
    let declarations =
      (try? AnnotationDeclaration(key, value: value)).map {
        [$0]
      } ?? []
    return modifier(ChillAnnotationModifier(declarations: declarations))
  }

  /// Adds a deterministic homogeneous map as one atomic annotation scope.
  public func annotation<Value: AnnotationValueConvertible>(
    _ values: [String: Value]
  ) -> some View {
    let declarations = values.keys.sorted().compactMap { name in
      values[name].flatMap { value in
        try? AnnotationDeclaration(name, value: value)
      }
    }
    return modifier(ChillAnnotationModifier(declarations: declarations))
  }
}
