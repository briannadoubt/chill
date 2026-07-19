import Foundation
import Testing
import os

@_spi(TraceIntegration) @testable import ChillCore
@testable import ChillNetworking

private struct StubResponse {
  let statusCode: Int
  let data: Data
  let error: (any Error)?

  static let ok = StubResponse(
    statusCode: 204,
    data: Data(),
    error: nil
  )
}

private final class StubURLProtocol: URLProtocol, @unchecked Sendable {
  nonisolated(unsafe) static var handler: ((URLRequest) -> StubResponse)?

  override class func canInit(with _: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest {
    request
  }

  override func startLoading() {
    guard let response = Self.handler?(request) else {
      client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
      return
    }
    if let error = response.error {
      client?.urlProtocol(self, didFailWithError: error)
      return
    }
    let http = HTTPURLResponse(
      url: request.url!,
      statusCode: response.statusCode,
      httpVersion: "HTTP/1.1",
      headerFields: [:]
    )!
    client?.urlProtocol(
      self,
      didReceive: http,
      cacheStoragePolicy: .notAllowed
    )
    client?.urlProtocol(self, didLoad: response.data)
    client?.urlProtocolDidFinishLoading(self)
  }

  override func stopLoading() {}
}

private final class TestTraceHandle: ChillTraceSpanHandle, @unchecked Sendable {
  struct State {
    var attributes: [String: ChillTraceAttributeValue]
    var status: ChillTraceSpanStatus?
  }

  let context: TraceContext
  private let state: OSAllocatedUnfairLock<State>

  init(
    context: TraceContext,
    attributes: [String: ChillTraceAttributeValue]
  ) {
    self.context = context
    state = OSAllocatedUnfairLock(
      initialState: State(attributes: attributes, status: nil)
    )
  }

  func setAttributes(_ attributes: [String: ChillTraceAttributeValue]) {
    state.withLock { $0.attributes.merge(attributes) { _, new in new } }
  }

  func end(status: ChillTraceSpanStatus) {
    state.withLock { $0.status = status }
  }

  var snapshot: State { state.withLock { $0 } }
}

private final class TestTraceProvider: ChillTraceProvider,
  @unchecked Sendable
{
  struct Started {
    let request: ChillTraceSpanRequest
    let handle: TestTraceHandle
  }

  private let state = OSAllocatedUnfairLock(initialState: [Started]())

  func startSpan(_ request: ChillTraceSpanRequest) -> any ChillTraceSpanHandle {
    let number = state.withLock { $0.count + 1 }
    let spanID = String(format: "%016llx", number)
    let context = try! TraceContext(
      traceID: request.parent?.traceID
        ?? "4bf92f3577b34da6a3ce929d0e0e4736",
      spanID: spanID,
      parentSpanID: request.parent?.spanID,
      traceFlags: request.parent?.traceFlags ?? 1,
      traceState: request.parent?.traceState ?? "vendor=value"
    )
    let handle = TestTraceHandle(
      context: context,
      attributes: request.attributes
    )
    state.withLock { $0.append(Started(request: request, handle: handle)) }
    return handle
  }

  var started: [Started] { state.withLock { $0 } }
}

@Suite("Apple networking trace propagation", .serialized)
struct NetworkingTests {
  @Test
  func trustedRequestsPropagateAndExportOnlyBoundedMetadata() async throws {
    let provider = TestTraceProvider()
    Chill.configureTraceProvider(provider)
    defer { ChillTraceRuntime.resetProvider() }

    let trustedURL = URL(string: "https://api.example.test")!
    let policy = try ChillNetworkPropagationPolicy(
      trustedOriginURLs: [trustedURL],
      baggageAllowlist: ["release.channel"],
      baggage: [
        "release.channel": "internal beta",
        "session.id": "must-not-leak",
      ]
    )
    let session = makeSession()
    defer { session.invalidateAndCancel() }
    let client = session.chill(policy)
    let captured = OSAllocatedUnfairLock<URLRequest?>(initialState: nil)
    StubURLProtocol.handler = { request in
      captured.withLock { $0 = request }
      return .ok
    }

    var request = URLRequest(
      url: URL(
        string: "https://api.example.test/private?token=secret"
      )!
    )
    request.httpMethod = "POST"
    request.setValue("Bearer secret", forHTTPHeaderField: "Authorization")
    request.httpBody = Data("private body".utf8)
    let (_, response) = try await client.data(for: request)

    #expect((response as? HTTPURLResponse)?.statusCode == 204)
    let sent = try #require(captured.withLock { $0 })
    #expect(
      sent.value(forHTTPHeaderField: "traceparent")
        == "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000001-01"
    )
    #expect(sent.value(forHTTPHeaderField: "tracestate") == "vendor=value")
    #expect(
      sent.value(forHTTPHeaderField: "baggage")
        == "release.channel=internal%20beta"
    )

    let started = try #require(provider.started.first)
    #expect(started.request.kind == .client)
    #expect(started.request.name == "POST")
    let attributes = started.handle.snapshot.attributes
    #expect(attributes["http.request.method"] == .string("POST"))
    #expect(attributes["url.scheme"] == .string("https"))
    #expect(attributes["server.address"] == .string("api.example.test"))
    #expect(attributes["http.response.status_code"] == .integer(204))
    #expect(attributes["chill.duration.nano"] != nil)
    let rendered = String(describing: attributes)
    #expect(!rendered.contains("/private"))
    #expect(!rendered.contains("secret"))
    #expect(!rendered.contains("Bearer"))
  }

  @Test
  func untrustedDestinationsAndRedirectsLoseAllPropagationHeaders() throws {
    let context = try TraceContext(
      traceID: "4bf92f3577b34da6a3ce929d0e0e4736",
      spanID: "00f067aa0ba902b7",
      traceFlags: 1,
      traceState: "vendor=value"
    )
    let policy = try ChillNetworkPropagationPolicy(
      trustedOriginURLs: [URL(string: "https://api.example.test")!],
      baggageAllowlist: ["release.channel"],
      baggage: ["release.channel": "beta"]
    )
    var redirected = URLRequest(
      url: URL(string: "https://sub.api.example.test/path")!
    )
    redirected.setValue("attacker", forHTTPHeaderField: "traceparent")
    redirected.setValue("attacker", forHTTPHeaderField: "tracestate")
    redirected.setValue("session.id=secret", forHTTPHeaderField: "baggage")

    let prepared = NetworkHeaderPropagation.prepare(
      redirected,
      context: context,
      policy: policy
    )
    #expect(prepared.value(forHTTPHeaderField: "traceparent") == nil)
    #expect(prepared.value(forHTTPHeaderField: "tracestate") == nil)
    #expect(prepared.value(forHTTPHeaderField: "baggage") == nil)
  }

  @Test
  func nativeActionContextAutomaticallyParentsTheClientSpan() async throws {
    let provider = TestTraceProvider()
    Chill.configureTraceProvider(provider)
    defer { ChillTraceRuntime.resetProvider() }
    let session = makeSession()
    defer { session.invalidateAndCancel() }
    StubURLProtocol.handler = { _ in .ok }

    let task = try _ChillInstrumentation.withActionTrace(
      name: SemanticName("cat.adopt")
    ) {
      Task {
        try await session.chill().data(
          from: URL(string: "https://api.example.test/cats")!
        )
      }
    }
    _ = try await task.value

    #expect(provider.started.count == 2)
    let action = provider.started[0].handle.context
    let client = provider.started[1]
    #expect(client.request.kind == .client)
    #expect(client.request.parent == action)
    #expect(client.handle.context.traceID == action.traceID)
    #expect(client.handle.context.parentSpanID == action.spanID)
  }

  @Test
  func asyncUploadAndDownloadUseTheSameInstrumentationPath() async throws {
    let provider = TestTraceProvider()
    Chill.configureTraceProvider(provider)
    defer { ChillTraceRuntime.resetProvider() }
    let session = makeSession()
    defer { session.invalidateAndCancel() }
    StubURLProtocol.handler = { _ in
      StubResponse(statusCode: 200, data: Data("ok".utf8), error: nil)
    }
    let client = session.chill()
    var uploadRequest = URLRequest(
      url: URL(string: "https://api.example.test/upload")!
    )
    uploadRequest.httpMethod = "PUT"

    _ = try await client.upload(
      for: uploadRequest,
      from: Data("private".utf8)
    )
    let (downloaded, _) = try await client.download(
      for: URLRequest(
        url: URL(string: "https://api.example.test/download")!
      )
    )

    #expect(provider.started.map(\.request.name) == ["PUT", "GET"])
    #expect(try Data(contentsOf: downloaded) == Data("ok".utf8))
  }

  @Test
  func baggageIsAllowlistedSortedAndBounded() throws {
    let oversized = String(repeating: "x", count: 129)
    let policy = ChillNetworkPropagationPolicy(
      trustedOrigins: [],
      baggageAllowlist: ["allowed", "too-long", "z"],
      baggage: [
        "z": "last",
        "allowed": "internal beta",
        "too-long": oversized,
        "user.id": "secret",
      ]
    )
    #expect(
      policy.baggageHeader
        == "allowed=internal%20beta,z=last"
    )
  }

  @Test
  func failuresEndTheSpanWithoutSerializingErrorDescriptions() async throws {
    let provider = TestTraceProvider()
    Chill.configureTraceProvider(provider)
    defer { ChillTraceRuntime.resetProvider() }
    let session = makeSession()
    defer { session.invalidateAndCancel() }
    StubURLProtocol.handler = { _ in
      StubResponse(
        statusCode: 0,
        data: Data(),
        error: URLError(.cannotConnectToHost)
      )
    }

    await #expect(throws: URLError.self) {
      _ = try await session.chill().data(
        from: URL(string: "https://offline.example.test/private")!
      )
    }
    let started = try #require(provider.started.first)
    #expect(started.handle.snapshot.status == .error)
    #expect(
      started.handle.snapshot.attributes["error.type"]
        == .string("URLError.-1004")
    )
    #expect(
      !String(describing: started.handle.snapshot.attributes)
        .contains("offline.example.test/private")
    )
  }

  private func makeSession() -> URLSession {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [StubURLProtocol.self]
    return URLSession(configuration: configuration)
  }
}
