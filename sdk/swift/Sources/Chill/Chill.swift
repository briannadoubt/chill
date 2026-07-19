@_exported import ChillCore
@_exported import ChillExport
@_exported import ChillNetworking
@_exported import ChillSwiftUI

#if canImport(UIKit)
  @_exported import ChillUIKit
#endif

/// Instruments the complete execution of a function as an activity.
///
/// The macro wraps only the function body. Its declaration, generic
/// constraints, isolation, error behavior, and return type remain unchanged.
@attached(body)
public macro Activity(
  _ name: String,
  kind: ActivityKind = .domain,
  role: ActivityRole = .operation
) =
  #externalMacro(
    module: "ChillMacrosPlugin",
    type: "ActivityMacro"
  )

/// Emits one declarative event at the selected function lifecycle boundary.
///
/// Use `entered`, `succeeded`, or `terminal` for function instrumentation.
/// `observed` belongs to declarative UI state observation and is rejected here.
@attached(body)
public macro Event(
  _ name: String,
  eventClass: EventClass = .domain,
  severity: EventSeverity = .info,
  emission: EventEmission = .succeeded,
  deduplicationKey: String? = nil
) =
  #externalMacro(
    module: "ChillMacrosPlugin",
    type: "EventMacro"
  )
