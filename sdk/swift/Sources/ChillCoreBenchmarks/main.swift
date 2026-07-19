import ChillCore
import Dispatch
import Foundation

private struct FixedClock: ChillClock {
  func now() -> TimePoint {
    TimePoint(
      occurredAtUnixNano: 1_700_000_000_000_000_000,
      monotonicNano: 1_000
    )
  }
}

private struct FixedIDGenerator: RecordIDGenerating {
  private let value = try! RecordID("00000000-0000-7000-8000-000000000001")
  func nextRecordID() -> RecordID { value }
}

private func configuration(enabled: Bool) -> RuntimeConfiguration {
  try! RuntimeConfiguration(
    enabled: enabled,
    sessionID: SessionID("00000000-0000-7000-8000-000000000002"),
    bootID: BootID("00000000-0000-4000-8000-000000000003"),
    privacy: PrivacyPolicy(
      version: "benchmark-v1",
      analytics: .granted
    ),
    sampling: SamplingConfiguration(
      stableKey: "benchmark",
      salt: "benchmark-v1"
    )
  )
}

private let draft = try! BehaviorDraft(
  subjectID: SubjectID("00000000-0000-7000-8000-000000000004"),
  kind: .event,
  operation: .instant,
  name: SemanticName("benchmark.event"),
  payload: .event(
    EventPayload(
      eventClass: .performance,
      severity: .info,
      emission: .observed
    )
  )
)

private func measure(iterations: Int, _ body: () -> Void) -> Double {
  let started = DispatchTime.now().uptimeNanoseconds
  for _ in 0..<iterations { body() }
  let elapsed = DispatchTime.now().uptimeNanoseconds - started
  return Double(elapsed) / Double(iterations)
}

private func percentile(_ values: [Double], _ probability: Double) -> Double {
  let sorted = values.sorted()
  let index = min(
    sorted.count - 1,
    max(0, Int(ceil(Double(sorted.count) * probability)) - 1)
  )
  return sorted[index]
}

let iterations = CommandLine.arguments.dropFirst().first.flatMap(Int.init) ?? 1_000_000
let sampleCount = 20
let iterationsPerSample = max(1, iterations / sampleCount)
let disabled = ChillRuntime(
  configuration: configuration(enabled: false),
  sink: NoOpRecordSink(),
  clock: FixedClock(),
  idGenerator: FixedIDGenerator()
)
let enabled = ChillRuntime(
  configuration: configuration(enabled: true),
  sink: NoOpRecordSink(),
  clock: FixedClock(),
  idGenerator: FixedIDGenerator()
)

var disabledBuilderExecutions = 0
var emitted = 0
var disabledSamples: [Double] = []
var enabledSamples: [Double] = []
for _ in 0..<sampleCount {
  disabledSamples.append(
    measure(iterations: iterationsPerSample) {
      if disabled.record(
        captureClass: .analytics,
        {
          disabledBuilderExecutions += 1
          return draft
        })
      {
        emitted += 1
      }
    })
  enabledSamples.append(
    measure(iterations: iterationsPerSample) {
      if enabled.record(captureClass: .analytics, { draft }) {
        emitted += 1
      }
    })
}
let disabledNanoseconds =
  disabledSamples.reduce(0, +) / Double(disabledSamples.count)
let enabledNanoseconds =
  enabledSamples.reduce(0, +) / Double(enabledSamples.count)
let disabledP99 = percentile(disabledSamples, 0.99)
let enabledP99 = percentile(enabledSamples, 0.99)

guard disabledBuilderExecutions == 0, disabledP99 <= 100 else {
  fputs("disabled capture exceeded the Apple release hot-path budget\n", stderr)
  exit(1)
}

let report: [String: Any] = [
  "iterations": iterationsPerSample * sampleCount,
  "samples": sampleCount,
  "disabled_nanoseconds_per_call": disabledNanoseconds,
  "disabled_p99_nanoseconds_per_call": disabledP99,
  "disabled_builder_executions": disabledBuilderExecutions,
  "enabled_nanoseconds_per_call": enabledNanoseconds,
  "enabled_p99_nanoseconds_per_call": enabledP99,
  "emitted": emitted,
  "host_presubmit_only": true,
]
let data = try! JSONSerialization.data(
  withJSONObject: report,
  options: [.prettyPrinted, .sortedKeys]
)
print(String(decoding: data, as: UTF8.self))
