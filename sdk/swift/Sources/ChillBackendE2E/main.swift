import Chill
import Foundation

private enum BackendE2EError: Error {
  case missingEnvironment(String)
  case invalidEndpoint
  case collectionPolicyDidNotApply
  case exportDidNotEmpty(ChillFlushResult)
  case missingAcknowledgement
}

@Event("validation.backend_e2e", emission: .succeeded)
private func exerciseDeclarativeEvent() -> String {
  "accepted"
}

@main
private struct ChillBackendE2E {
  static func main() async throws {
    let environment = ProcessInfo.processInfo.environment
    guard let endpointValue = environment["CHILL_OTLP_ENDPOINT"] else {
      throw BackendE2EError.missingEnvironment("CHILL_OTLP_ENDPOINT")
    }
    guard let sdkKey = environment["CHILL_API_KEY"] else {
      throw BackendE2EError.missingEnvironment("CHILL_API_KEY")
    }
    guard let endpoint = URL(string: endpointValue) else {
      throw BackendE2EError.invalidEndpoint
    }

    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "chill-backend-e2e-\(UUID().uuidString.lowercased())",
      isDirectory: true
    )
    defer { try? FileManager.default.removeItem(at: directory) }
    let pipeline = try ChillOfflinePipeline(
      directory: directory,
      configuration: ChillOTLPConfiguration(
        endpoint: endpoint,
        headers: ["authorization": "Bearer \(sdkKey)"],
        resourceAttributes: [
          "service.name": "chill-backend-e2e",
          "deployment.environment.name": "test",
        ],
        offline: try ChillOfflineConfiguration(flushInterval: 0.01),
        allowInsecureLocalhost: true
      )
    )
    let runtime = ChillRuntime(
      configuration: try RuntimeConfiguration(
        sessionID: SessionID(
          UUIDv7.generate().uuidString.lowercased()
        ),
        bootID: BootID(UUID().uuidString.lowercased()),
        privacy: PrivacyPolicy(
          version: "backend-e2e-v1",
          analytics: .granted,
          diagnostic: .granted
        ),
        sampling: SamplingConfiguration(
          stableKey: "backend-e2e-installation",
          salt: "backend-e2e-v1"
        )
      ),
      sink: pipeline
    )
    var collectionComponents = URLComponents(
      url: endpoint,
      resolvingAgainstBaseURL: false
    )
    collectionComponents?.path = "/v1/chill/collection-state"
    collectionComponents?.query = nil
    guard let collectionEndpoint = collectionComponents?.url else {
      throw BackendE2EError.invalidEndpoint
    }
    let policyClient = try ChillCollectionPolicyClient(
      endpoint: collectionEndpoint,
      headers: ["authorization": "Bearer \(sdkKey)"],
      allowInsecureLocalhost: true
    )
    let policySynchronizer = ChillCollectionPolicySynchronizer(
      runtime: runtime,
      client: policyClient
    )
    guard try await policySynchronizer.refresh() == .applied else {
      throw BackendE2EError.collectionPolicyDidNotApply
    }
    Chill.configure(runtime)
    defer { Chill.disable() }

    guard exerciseDeclarativeEvent() == "accepted" else {
      fatalError("instrumented function changed behavior")
    }
    let result = await pipeline.shutdown()
    guard result == .emptied else {
      throw BackendE2EError.exportDidNotEmpty(result)
    }
    let metrics = await pipeline.metrics()
    guard metrics.acknowledgedRecords == 1,
      metrics.queuedRecords == 0,
      metrics.exportFailures == 0
    else {
      throw BackendE2EError.missingAcknowledgement
    }
    print(
      "{\"acknowledgedRecords\":\(metrics.acknowledgedRecords),"
        + "\"exportAttempts\":\(metrics.exportAttempts),"
        + "\"queuedRecords\":\(metrics.queuedRecords)}"
    )
  }
}
