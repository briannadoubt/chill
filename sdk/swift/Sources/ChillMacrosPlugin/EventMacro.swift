import SwiftDiagnostics
import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

public struct EventMacro: BodyMacro {
  public static func expansion(
    of node: AttributeSyntax,
    providingBodyFor declaration: some DeclSyntaxProtocol & WithOptionalCodeBlockSyntax,
    in context: some MacroExpansionContext
  ) throws -> [CodeBlockItemSyntax] {
    guard
      let parsed = FunctionMacroArguments.parse(
        attribute: node,
        declaration: declaration,
        context: context
      )
    else {
      return originalBody(of: declaration)
    }
    if parsed.hasCase(label: "emission", named: "observed") {
      context.diagnose(
        Diagnostic(
          node: Syntax(node),
          message: ChillMacroDiagnostic.observedEventUnsupported
        )
      )
      return Array(parsed.originalBody.statements)
    }
    let eventClass = parsed.expression(
      label: "eventClass",
      default: ".domain"
    )
    let severity = parsed.expression(label: "severity", default: ".info")
    let emission = parsed.expression(
      label: "emission",
      default: ".succeeded"
    )
    let deduplicationKey = parsed.expression(
      label: "deduplicationKey",
      default: "nil"
    )
    let function = parsed.isAsync ? "withAsyncEvent" : "withEvent"
    return [
      CodeBlockItemSyntax(
        stringLiteral: """
          return \(parsed.invocationPrefix)_ChillInstrumentation.\(function)(
            name: \(parsed.nameExpression),
            eventClass: \(eventClass),
            severity: \(severity),
            emission: \(emission),
            deduplicationKey: \(deduplicationKey)
          ) \(parsed.closureHeader)
          \(parsed.originalStatements)
          }
          """
      )
    ]
  }
}
