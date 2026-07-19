import Foundation
import Testing

@testable import ChillCore
@testable import ChillExport

private final class CollectionPolicyURLProtocol: URLProtocol,
  @unchecked Sendable
{
  nonisolated(unsafe) static var response: (Int, Data)?

  override class func canInit(with _: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest {
    request
  }

  override func startLoading() {
    guard let (status, body) = Self.response else {
      client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
      return
    }
    let response = HTTPURLResponse(
      url: request.url!,
      statusCode: status,
      httpVersion: "HTTP/1.1",
      headerFields: ["Content-Type": "application/json"]
    )!
    client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
    client?.urlProtocol(self, didLoad: body)
    client?.urlProtocolDidFinishLoading(self)
  }

  override func stopLoading() {}
}

@Suite("Collection policy propagation", .serialized)
struct CollectionPolicyClientTests {
  @Test
  func fetchesAndAppliesRemoteNarrowingPolicy() async throws {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [CollectionPolicyURLProtocol.self]
    let session = URLSession(configuration: configuration)
    defer { session.invalidateAndCancel() }
    let body = #"""
      {
        "revision": 2,
        "policy_version": "privacy-v1",
        "enabled": true,
        "disabled_capture_classes": ["replay"],
        "effective_at_unix_nano": 123
      }
      """#
    CollectionPolicyURLProtocol.response = (
      200,
      Data(body.utf8)
    )
    let client = try ChillCollectionPolicyClient(
      endpoint: URL(string: "http://localhost:4318/v1/chill/collection-state")!,
      headers: ["Authorization": "Bearer secret"],
      allowInsecureLocalhost: true,
      session: session
    )
    let runtime = ChillRuntime(
      configuration: try RuntimeConfiguration(
        sessionID: SessionID("session-1"),
        bootID: BootID("boot-1"),
        privacy: PrivacyPolicy(
          version: "privacy-v1",
          analytics: .granted,
          replay: .granted
        ),
        sampling: SamplingConfiguration(
          stableKey: "test", salt: "test-salt",
          behavior: .all, replay: .all
        )
      ),
      sink: NoOpRecordSink()
    )
    let synchronizer = ChillCollectionPolicySynchronizer(
      runtime: runtime,
      client: client
    )

    #expect(try await synchronizer.refresh() == .applied)
    #expect(runtime.isCaptureEnabled(for: .analytics))
    #expect(!runtime.isCaptureEnabled(for: .replay))
  }

  @Test
  func rejectsAuthenticationAndOversizedResponses() async throws {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [CollectionPolicyURLProtocol.self]
    let session = URLSession(configuration: configuration)
    defer { session.invalidateAndCancel() }
    let client = try ChillCollectionPolicyClient(
      endpoint: URL(string: "http://127.0.0.1:4318/v1/chill/collection-state")!,
      headers: [:],
      allowInsecureLocalhost: true,
      session: session
    )
    CollectionPolicyURLProtocol.response = (401, Data())
    await #expect(throws: ChillCollectionPolicyError.unauthorized) {
      try await client.fetch()
    }
    CollectionPolicyURLProtocol.response = (200, Data(repeating: 0, count: 65 * 1_024))
    await #expect(throws: ChillCollectionPolicyError.responseTooLarge) {
      try await client.fetch()
    }
    let invalidBody = #"""
      {
        "revision": 0,
        "policy_version": "bad version",
        "enabled": true,
        "disabled_capture_classes": [],
        "effective_at_unix_nano": 0
      }
      """#
    CollectionPolicyURLProtocol.response = (200, Data(invalidBody.utf8))
    await #expect(throws: ChillCollectionPolicyError.invalidResponse) {
      try await client.fetch()
    }
  }
}
