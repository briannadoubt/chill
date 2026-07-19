#if canImport(AppKit)
  import AppKit
  import ChillCore
  import ChillReplay
  import Dispatch
  import Foundation

  @main
  @MainActor
  struct ChillReplayBenchmarks {
    static func main() throws {
      let runtime = try ChillRuntime(
        configuration: RuntimeConfiguration(
          sessionID: SessionID("benchmark-session"),
          bootID: BootID("benchmark-boot"),
          privacy: PrivacyPolicy(
            version: "benchmark-policy-v1",
            replay: .granted
          ),
          sampling: SamplingConfiguration(
            stableKey: "benchmark-subject",
            salt: "benchmark-salt-v1",
            replay: .all
          )
        ),
        sink: NoOpRecordSink()
      )
      let root = NSView(
        frame: NSRect(x: 0, y: 0, width: 390, height: 844)
      )
      for index in 0..<100 {
        let view: NSView
        switch index % 10 {
        case 0:
          let secure = NSSecureTextField(
            frame: NSRect(x: 0, y: 0, width: 180, height: 20)
          )
          secure.stringValue = "SENSITIVE-BENCHMARK-CANARY"
          view = secure
        case 1:
          view = NSImageView(
            frame: NSRect(x: 0, y: 0, width: 44, height: 44)
          )
        default:
          view = NSTextField(
            labelWithString: "SENSITIVE-BENCHMARK-CANARY-\(index)"
          )
        }
        view.frame.origin = NSPoint(
          x: CGFloat((index % 5) * 76),
          y: CGFloat((index / 5) * 40)
        )
        root.addSubview(view)
      }
      let window = NSWindow(
        contentRect: root.bounds,
        styleMask: [.borderless],
        backing: .buffered,
        defer: false
      )
      window.contentView = root

      for _ in 0..<100 {
        _ = AppKitReplayTree.snapshot(
          windows: [window],
          runtime: runtime,
          maximumNodes: 1_024
        )
      }
      var samples: [UInt64] = []
      samples.reserveCapacity(2_000)
      for _ in 0..<2_000 {
        let start = DispatchTime.now().uptimeNanoseconds
        let observation = AppKitReplayTree.snapshot(
          windows: [window],
          runtime: runtime,
          maximumNodes: 1_024
        )
        let end = DispatchTime.now().uptimeNanoseconds
        guard observation?.nodes.count == 101 else {
          fputs("replay benchmark produced an incomplete frame\n", stderr)
          Foundation.exit(2)
        }
        samples.append(end - start)
      }
      samples.sort()
      let p95 = milliseconds(samples[percentileIndex(0.95, count: samples.count)])
      let p99 = milliseconds(samples[percentileIndex(0.99, count: samples.count)])
      let average = milliseconds(samples.reduce(0, +) / UInt64(samples.count))
      let report: [String: Any] = [
        "replay_nodes": 101,
        "iterations": samples.count,
        "average_milliseconds": average,
        "p95_milliseconds": p95,
        "p99_milliseconds": p99,
        "host_presubmit_only": true,
      ]
      let encoded = try! JSONSerialization.data(
        withJSONObject: report,
        options: [.prettyPrinted, .sortedKeys]
      )
      print(String(decoding: encoded, as: UTF8.self))
      guard p95 <= 1, p99 <= 2 else {
        fputs("replay capture exceeded the Apple release budget\n", stderr)
        Foundation.exit(1)
      }
    }

    private static func percentileIndex(
      _ percentile: Double,
      count: Int
    ) -> Int {
      min(count - 1, max(0, Int((Double(count) * percentile).rounded(.up)) - 1))
    }

    private static func milliseconds(_ nanoseconds: UInt64) -> Double {
      Double(nanoseconds) / 1_000_000
    }
  }
#else
  import Foundation

  @main
  struct ChillReplayBenchmarks {
    static func main() {
      print("ChillReplayBenchmarks requires AppKit")
    }
  }
#endif
