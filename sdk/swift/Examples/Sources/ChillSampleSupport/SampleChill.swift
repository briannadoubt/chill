import Chill
import ChillReplay
import Foundation

public enum SampleChill {
  @MainActor
  public static func configure(
    application: String,
    environment: [String: String] = ProcessInfo.processInfo.environment
  ) throws {
    let baseDirectory =
      FileManager.default.urls(
        for: .applicationSupportDirectory,
        in: .userDomainMask
      ).first ?? FileManager.default.temporaryDirectory
    let root = baseDirectory.appendingPathComponent(
      "dev.chill.examples/\(application)",
      isDirectory: true
    )
    guard
      let endpoint = URL(
        string: environment["CHILL_OTLP_ENDPOINT"]
          ?? "https://collector.example.invalid/v1/logs"
      )
    else {
      throw ChillExportConfigurationError.invalidEndpoint
    }
    var headers: [String: String] = [:]
    if let key = environment["CHILL_API_KEY"], !key.isEmpty {
      headers["authorization"] = "Bearer \(key)"
    }
    let allowInsecureLocalhost =
      environment["CHILL_ALLOW_INSECURE_LOCALHOST"] == "1"
    let exporter = try ChillOfflinePipeline(
      directory: root.appendingPathComponent("events", isDirectory: true),
      configuration: ChillOTLPConfiguration(
        endpoint: endpoint,
        headers: headers,
        resourceAttributes: [
          "service.name": application,
          "deployment.environment": "sample",
        ],
        allowInsecureLocalhost: allowInsecureLocalhost
      )
    )
    let runtime = ChillRuntime(
      configuration: try RuntimeConfiguration(
        sessionID: SessionID(
          UUIDv7.generate().uuidString.lowercased()
        ),
        bootID: BootID(UUID().uuidString.lowercased()),
        privacy: PrivacyPolicy(
          version: "privacy-v1",
          analytics: .granted,
          diagnostic: .granted,
          replay: .granted,
          annotationAllowlist: [
            "account.tier": .internalData,
            "app.area": .internalData,
            "build.channel": .internalData,
            "cat.id": .pseudonymousIdentifier,
          ]
        ),
        sampling: SamplingConfiguration(
          stableKey: "sample-installation",
          salt: "sample-v1",
          replay: .all
        )
      ),
      sink: exporter
    )
    Chill.configure(runtime)

    try ChillSessionReplay.configure(
      .standard(
        directory: root.appendingPathComponent("replay", isDirectory: true),
        keyIdentifier: application
      )
    )
  }
}
