import ChillMacrosPlugin
import SwiftSyntax
import SwiftSyntaxMacros
import SwiftSyntaxMacrosTestSupport
import Testing

private let chillMacros: [String: Macro.Type] = [
  "Activity": ActivityMacro.self,
  "Event": EventMacro.self,
]

@Test("Activity preserves a generic synchronous declaration")
func activityExpansion() {
  assertMacroExpansion(
    """
    @Activity("repository.load", kind: .storage)
    func load<Value>(_ value: Value) -> Value where Value: Sendable {
      value
    }
    """,
    expandedSource: """

      func load<Value>(_ value: Value) -> Value where Value: Sendable {
        return _ChillInstrumentation.withActivity(
          name: "repository.load",
          kind: .storage,
          role: .operation
        ) {
          value
        }
      }
      """,
    macros: chillMacros,
    indentationWidth: .spaces(2)
  )
}

@Test("Event preserves async and throwing effects")
func eventExpansion() {
  assertMacroExpansion(
    """
    @Event(
      "cat.loaded",
      eventClass: .domain,
      severity: .debug,
      emission: .terminal,
      deduplicationKey: "cat-42"
    )
    func loadCat() async throws -> String {
      try await fetchCat()
    }
    """,
    expandedSource: """

      func loadCat() async throws -> String {
        return try await _ChillInstrumentation.withAsyncEvent(
          name: "cat.loaded",
          eventClass: .domain,
          severity: .debug,
          emission: .terminal,
          deduplicationKey: "cat-42"
        ) {
          try await fetchCat()
        }
      }
      """,
    macros: chillMacros,
    indentationWidth: .spaces(2)
  )
}

@Test("Typed throws keeps the original error context")
func typedThrowsExpansion() {
  assertMacroExpansion(
    """
    @Activity("repository.typed")
    func load() throws(RepositoryError) -> Cat {
      throw .unavailable
    }
    """,
    expandedSource: """

      func load() throws(RepositoryError) -> Cat {
        return try _ChillInstrumentation.withActivity(
          name: "repository.typed",
          kind: .domain,
          role: .operation
        ) { () throws(RepositoryError) in
          throw .unavailable
        }
      }
      """,
    macros: chillMacros,
    indentationWidth: .spaces(2)
  )
}

@Test("Invalid semantic names are compile-time errors")
func invalidNameDiagnostic() {
  assertMacroExpansion(
    """
    @Activity("Cat Loaded")
    func loadCat() {}
    """,
    expandedSource: """

      func loadCat() {}
      """,
    diagnostics: [
      DiagnosticSpec(
        message:
          "semantic names must be at most 128 UTF-8 bytes and match [a-z][a-z0-9_]*(?:[.-][a-z0-9_]+)*",
        line: 1,
        column: 11
      )
    ],
    macros: chillMacros,
    indentationWidth: .spaces(2)
  )
}

@Test("Observed events are reserved for declarative state")
func observedEventDiagnostic() {
  assertMacroExpansion(
    """
    @Event("cat.changed", emission: .observed)
    func updateCat() {}
    """,
    expandedSource: """

      func updateCat() {}
      """,
    diagnostics: [
      DiagnosticSpec(
        message:
          "observed events belong to declarative UI state and cannot instrument a function body",
        line: 1,
        column: 1
      )
    ],
    macros: chillMacros,
    indentationWidth: .spaces(2)
  )
}

@Test("Names cannot depend on runtime interpolation")
func interpolatedNameDiagnostic() {
  assertMacroExpansion(
    """
    @Event("cat.\\(identifier)")
    func updateCat() {}
    """,
    expandedSource: """

      func updateCat() {}
      """,
    diagnostics: [
      DiagnosticSpec(
        message: "the semantic name must be a non-interpolated string literal",
        line: 1,
        column: 8
      )
    ],
    macros: chillMacros,
    indentationWidth: .spaces(2)
  )
}
