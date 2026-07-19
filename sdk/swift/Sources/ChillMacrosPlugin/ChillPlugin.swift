import SwiftCompilerPlugin
import SwiftSyntaxMacros

@main
struct ChillPlugin: CompilerPlugin {
  let providingMacros: [Macro.Type] = [
    ActivityMacro.self,
    EventMacro.self,
  ]
}
