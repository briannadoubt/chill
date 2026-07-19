/// Package SPI carried by zero-content SwiftUI probes. Replay adapters use the
/// probe's native geometry to enforce monotone privacy policy without reading
/// SwiftUI labels, state, accessibility text, or view values.
@MainActor
package protocol ReplayPolicyBoundary: AnyObject {
  var chillReplayPolicy: ViewCapturePolicy { get }
}
