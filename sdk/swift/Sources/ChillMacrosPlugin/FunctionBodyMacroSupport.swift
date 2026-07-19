import SwiftDiagnostics
import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

enum ChillMacroDiagnostic: String, DiagnosticMessage {
  case functionOnly
  case missingName
  case staticNameRequired
  case invalidName
  case observedEventUnsupported

  var message: String {
    switch self {
    case .functionOnly:
      "Chill body macros can only be attached to functions"
    case .missingName:
      "a semantic name is required"
    case .staticNameRequired:
      "the semantic name must be a non-interpolated string literal"
    case .invalidName:
      "semantic names must be at most 128 UTF-8 bytes and match [a-z][a-z0-9_]*(?:[.-][a-z0-9_]+)*"
    case .observedEventUnsupported:
      "observed events belong to declarative UI state and cannot instrument a function body"
    }
  }

  var diagnosticID: MessageID {
    MessageID(domain: "dev.chill.macros", id: rawValue)
  }

  var severity: DiagnosticSeverity { .error }
}

struct FunctionMacroArguments {
  let function: FunctionDeclSyntax
  let originalBody: CodeBlockSyntax
  let nameExpression: String
  let arguments: LabeledExprListSyntax

  static func parse(
    attribute: AttributeSyntax,
    declaration: some DeclSyntaxProtocol & WithOptionalCodeBlockSyntax,
    context: some MacroExpansionContext
  ) -> Self? {
    guard let function = declaration.as(FunctionDeclSyntax.self),
      let body = function.body
    else {
      context.diagnose(
        Diagnostic(node: Syntax(declaration), message: ChillMacroDiagnostic.functionOnly)
      )
      return nil
    }
    guard case .argumentList(let arguments) = attribute.arguments,
      let first = arguments.first
    else {
      context.diagnose(
        Diagnostic(node: Syntax(attribute), message: ChillMacroDiagnostic.missingName)
      )
      return nil
    }
    guard let name = staticString(first.expression) else {
      context.diagnose(
        Diagnostic(
          node: Syntax(first.expression),
          message: ChillMacroDiagnostic.staticNameRequired
        )
      )
      return nil
    }
    guard isValidSemanticName(name) else {
      context.diagnose(
        Diagnostic(
          node: Syntax(first.expression),
          message: ChillMacroDiagnostic.invalidName
        )
      )
      return nil
    }
    return Self(
      function: function,
      originalBody: body,
      nameExpression: first.expression.trimmedDescription,
      arguments: arguments
    )
  }

  var invocationPrefix: String {
    let effects = function.signature.effectSpecifiers
    return switch (effects?.throwsClause != nil, effects?.asyncSpecifier != nil) {
    case (true, true): "try await "
    case (true, false): "try "
    case (false, true): "await "
    case (false, false): ""
    }
  }

  var isAsync: Bool {
    function.signature.effectSpecifiers?.asyncSpecifier != nil
  }

  var closureHeader: String {
    guard let type = function.signature.effectSpecifiers?.throwsClause?.type
    else {
      return "{"
    }
    let asyncEffect = isAsync ? " async" : ""
    return "{ ()\(asyncEffect) throws(\(type.trimmedDescription)) in"
  }

  var originalStatements: String {
    originalBody.statements.trimmedDescription
  }

  func expression(label: String, default defaultValue: String) -> String {
    arguments.first { argument in
      argument.label?.text == label
    }?.expression.trimmedDescription ?? defaultValue
  }

  func hasCase(label: String, named caseName: String) -> Bool {
    guard
      let expression = arguments.first(where: {
        $0.label?.text == label
      })?.expression
    else {
      return false
    }
    let value = expression.trimmedDescription
    return value == ".\(caseName)" || value.hasSuffix(".\(caseName)")
  }
}

func originalBody(
  of declaration: some DeclSyntaxProtocol & WithOptionalCodeBlockSyntax
) -> [CodeBlockItemSyntax] {
  guard let body = declaration.body else { return [] }
  return Array(body.statements)
}

private func staticString(_ expression: ExprSyntax) -> String? {
  guard let literal = expression.as(StringLiteralExprSyntax.self),
    literal.openingPounds == nil,
    literal.openingQuote.tokenKind == .stringQuote,
    literal.segments.count == 1,
    let segment = literal.segments.first?.as(StringSegmentSyntax.self)
  else {
    return nil
  }
  return segment.content.text
}

private func isValidSemanticName(_ value: String) -> Bool {
  guard value.utf8.count <= 128,
    let first = value.first,
    first.isASCII,
    first.isLowercase
  else {
    return false
  }
  var previousWasSeparator = false
  for character in value.dropFirst() {
    if character == "." || character == "-" {
      if previousWasSeparator { return false }
      previousWasSeparator = true
      continue
    }
    guard character.isASCII,
      character.isLowercase || character.isNumber || character == "_"
    else {
      return false
    }
    previousWasSeparator = false
  }
  return !previousWasSeparator
}
