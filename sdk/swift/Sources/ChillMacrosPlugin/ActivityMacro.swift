import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

public struct ActivityMacro: BodyMacro {
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
    let kind = parsed.expression(label: "kind", default: ".domain")
    let role = parsed.expression(label: "role", default: ".operation")
    let function = parsed.isAsync ? "withAsyncActivity" : "withActivity"
    return [
      CodeBlockItemSyntax(
        stringLiteral: """
          return \(parsed.invocationPrefix)_ChillInstrumentation.\(function)(
            name: \(parsed.nameExpression),
            kind: \(kind),
            role: \(role)
          ) \(parsed.closureHeader)
          \(parsed.originalStatements)
          }
          """
      )
    ]
  }
}
