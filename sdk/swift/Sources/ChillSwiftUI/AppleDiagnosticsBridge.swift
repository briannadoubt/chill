import ChillCore
import os

#if canImport(MetricKit) && (os(iOS) || os(macOS))
  import Foundation
  import MetricKit

  private protocol AppleDiagnosticsInstallation: AnyObject, Sendable {}

  enum AppleDiagnosticsBridge {
    private static let installation = OSAllocatedUnfairLock(
      initialState: (any AppleDiagnosticsInstallation)?.none
    )

    static func installIfNeeded() {
      guard Chill.currentRuntime()?.isCaptureEnabled(for: .diagnostic) == true
      else {
        return
      }

      installation.withLock { installation in
        guard installation == nil else { return }
        if #available(iOS 27.0, macOS 27.0, *) {
          installation = ModernAppleDiagnosticsInstallation()
        } else {
          installation = LegacyAppleDiagnosticsInstallation()
        }
      }
    }

    fileprivate static func emit(
      _ rawName: String,
      eventClass: EventClass,
      severity: EventSeverity,
      durationNano: UInt64? = nil
    ) {
      guard let name = try? SemanticName(rawName) else { return }
      ChillUIInstrumentation.observedEvent(
        name: name,
        eventClass: eventClass,
        severity: severity,
        captureClass: .diagnostic,
        durationNano: durationNano,
        context: SwiftUIFactContext(
          annotations: .empty,
          page: nil,
          policy: .standard
        )
      )
    }
  }

  @available(iOS 27.0, macOS 27.0, *)
  private final class ModernAppleDiagnosticsInstallation:
    AppleDiagnosticsInstallation,
    @unchecked Sendable
  {
    private let manager: MetricManager
    private let metricTask: Task<Void, Never>
    private let diagnosticTask: Task<Void, Never>

    init() {
      let manager = MetricManager()
      self.manager = manager
      metricTask = Task.detached {
        for await _ in manager.metricReports {
          AppleDiagnosticsBridge.emit(
            "app.performance.reported",
            eventClass: .performance,
            severity: .info
          )
        }
      }
      diagnosticTask = Task.detached {
        for await report in manager.diagnosticReports {
          Self.receive(report.result)
        }
      }
    }

    deinit {
      metricTask.cancel()
      diagnosticTask.cancel()
    }

    private static func receive(_ result: DiagnosticResult) {
      switch result {
      case .crash:
        AppleDiagnosticsBridge.emit(
          "app.crash.detected",
          eventClass: .crash,
          severity: .fatal
        )
      case .hang(let diagnostic):
        AppleDiagnosticsBridge.emit(
          "app.hang.detected",
          eventClass: .performance,
          severity: .warn,
          durationNano: diagnostic.hangDuration.clampedNanoseconds
        )
      case .cpuException(let diagnostic):
        AppleDiagnosticsBridge.emit(
          "app.cpu_exception.detected",
          eventClass: .performance,
          severity: .warn,
          durationNano: diagnostic.totalCPUTime.clampedNanoseconds
        )
      case .diskWriteException:
        AppleDiagnosticsBridge.emit(
          "app.disk_write.detected",
          eventClass: .performance,
          severity: .warn
        )
      case .appLaunch(let diagnostic):
        AppleDiagnosticsBridge.emit(
          "app.launch.slow",
          eventClass: .performance,
          severity: .warn,
          durationNano: diagnostic.launchDuration.clampedNanoseconds
        )
      #if os(iOS)
        case .memoryException:
          AppleDiagnosticsBridge.emit(
            "app.memory_exception.detected",
            eventClass: .performance,
            severity: .warn
          )
      #endif
      @unknown default:
        break
      }
    }
  }

  @available(iOS, introduced: 17.0, obsoleted: 27.0)
  @available(macOS, introduced: 14.0, obsoleted: 27.0)
  private final class LegacyAppleDiagnosticsInstallation: NSObject,
    AppleDiagnosticsInstallation,
    MXMetricManagerSubscriber,
    @unchecked Sendable
  {
    private let manager: MXMetricManager

    override init() {
      manager = .shared
      super.init()
      manager.add(self)
    }

    deinit {
      manager.remove(self)
    }

    func didReceive(_: [MXMetricPayload]) {
      AppleDiagnosticsBridge.emit(
        "app.performance.reported",
        eventClass: .performance,
        severity: .info
      )
    }

    func didReceive(_ payloads: [MXDiagnosticPayload]) {
      for payload in payloads {
        for _ in payload.crashDiagnostics ?? [] {
          AppleDiagnosticsBridge.emit(
            "app.crash.detected",
            eventClass: .crash,
            severity: .fatal
          )
        }
        for diagnostic in payload.hangDiagnostics ?? [] {
          AppleDiagnosticsBridge.emit(
            "app.hang.detected",
            eventClass: .performance,
            severity: .warn,
            durationNano: diagnostic.hangDuration.clampedNanoseconds
          )
        }
        for diagnostic in payload.cpuExceptionDiagnostics ?? [] {
          AppleDiagnosticsBridge.emit(
            "app.cpu_exception.detected",
            eventClass: .performance,
            severity: .warn,
            durationNano: diagnostic.totalCPUTime.clampedNanoseconds
          )
        }
        for _ in payload.diskWriteExceptionDiagnostics ?? [] {
          AppleDiagnosticsBridge.emit(
            "app.disk_write.detected",
            eventClass: .performance,
            severity: .warn
          )
        }
        #if os(iOS)
          for diagnostic in payload.appLaunchDiagnostics ?? [] {
            AppleDiagnosticsBridge.emit(
              "app.launch.slow",
              eventClass: .performance,
              severity: .warn,
              durationNano: diagnostic.launchDuration.clampedNanoseconds
            )
          }
        #endif
      }
    }
  }

  extension Measurement where UnitType == UnitDuration {
    fileprivate var clampedNanoseconds: UInt64 {
      let seconds = converted(to: .seconds).value
      guard seconds.isFinite, seconds > 0 else { return 0 }
      let nanoseconds = seconds * 1_000_000_000
      guard nanoseconds < Double(UInt64.max) else { return .max }
      return UInt64(nanoseconds.rounded())
    }
  }
#else
  enum AppleDiagnosticsBridge {
    static func installIfNeeded() {}
  }
#endif
