import Chill
import Foundation
import Testing
import os

private final class ValidationSink: RecordSink {
  private let storage = OSAllocatedUnfairLock(
    initialState: [BehaviorRecord]()
  )

  func submit(_ record: BehaviorRecord) {
    storage.withLock { $0.append(record) }
  }

  var records: [BehaviorRecord] {
    storage.withLock { $0 }
  }
}

private struct GoldenRecord: Codable, Equatable {
  let captureClass: String
  let kind: String
  let name: String
  let operation: String
  let outcome: String?

  enum CodingKeys: String, CodingKey {
    case captureClass = "capture_class"
    case kind
    case name
    case operation
    case outcome
  }
}

@Event("validation.cat_loaded")
private func declareLoaded() {}

@Activity("validation.load_cat", kind: .storage)
private func loadCat() -> String {
  declareLoaded()
  return "cat-123"
}

@Activity("validation.stress", kind: .task)
private func stressUnit(_ value: Int) -> Int {
  value &+ 1
}

@Suite("Apple SDK release validation", .serialized)
struct ValidationTests {
  @Test("Macro telemetry matches the checked golden fixture")
  func goldenMacroTelemetry() throws {
    let sink = try configureValidationRuntime()
    defer { Chill.disable() }

    #expect(loadCat() == "cat-123")

    let actual = sink.records.map(GoldenRecord.init)
    let fixture = try #require(
      Bundle.module.url(
        forResource: "golden_macro_records",
        withExtension: "json"
      )
    )
    let expected = try JSONDecoder().decode(
      [GoldenRecord].self,
      from: Data(contentsOf: fixture)
    )
    #expect(actual == expected)
  }

  @Test("Concurrent macro capture preserves exact paired lifecycle records")
  func concurrentStress() async throws {
    let sink = try configureValidationRuntime()
    defer { Chill.disable() }
    let invocations = 2_000

    await withTaskGroup(of: Int.self) { group in
      for value in 0..<invocations {
        group.addTask { stressUnit(value) }
      }
      var completed = 0
      for await _ in group { completed += 1 }
      #expect(completed == invocations)
    }

    let records = sink.records
    #expect(records.count == invocations * 2)
    let grouped = Dictionary(grouping: records, by: \.subjectID)
    #expect(grouped.count == invocations)
    #expect(
      grouped.values.allSatisfy { records in
        records.map(\.operation).sorted(by: operationOrder) == [.start, .end]
      })
  }

  @Test("Disabled hot path never invokes the sink")
  func disabledStress() throws {
    let sink = try configureValidationRuntime()
    Chill.disable()

    var total = 0
    for value in 0..<100_000 {
      total &+= stressUnit(value)
    }

    #expect(total != 0)
    #expect(sink.records.isEmpty)
  }
}

extension GoldenRecord {
  init(_ record: BehaviorRecord) {
    let outcome: TerminalOutcome? =
      switch record.payload {
      case .activity(let payload): payload.outcome
      case .event(let payload): payload.outcome
      default: nil
      }
    self.init(
      captureClass: record.captureClass.rawValue,
      kind: record.kind.rawValue,
      name: record.name.rawValue,
      operation: record.operation.rawValue,
      outcome: outcome?.rawValue
    )
  }
}

private func configureValidationRuntime() throws -> ValidationSink {
  Chill.disable()
  let sink = ValidationSink()
  Chill.configure(
    ChillRuntime(
      configuration: try RuntimeConfiguration(
        sessionID: SessionID("validation-session"),
        bootID: BootID("validation-boot"),
        privacy: PrivacyPolicy(
          version: "validation-v1",
          analytics: .granted,
          diagnostic: .granted,
          replay: .granted
        ),
        sampling: SamplingConfiguration(
          stableKey: "validation-subject",
          salt: "validation-v1",
          replay: .all
        )
      ),
      sink: sink
    )
  )
  return sink
}

private func operationOrder(
  _ left: BehaviorOperation,
  _ right: BehaviorOperation
) -> Bool {
  func rank(_ operation: BehaviorOperation) -> Int {
    switch operation {
    case .start: 0
    case .end: 1
    default: 2
    }
  }
  return rank(left) < rank(right)
}
