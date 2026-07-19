@_spi(TraceIntegration) import ChillCore
import Dispatch
import Foundation

/// A modern URLSession facade that creates one standard client span per task
/// and applies Chill's exact-origin propagation policy immediately before send.
/// It never reads request/response bodies, authorization values, or URL paths.
public struct ChillURLSession: Sendable {
  public let session: URLSession
  public let propagation: ChillNetworkPropagationPolicy

  public init(
    session: URLSession = .shared,
    propagation: ChillNetworkPropagationPolicy = .none
  ) {
    self.session = session
    self.propagation = propagation
  }

  public func data(
    for request: URLRequest
  ) async throws -> (Data, URLResponse) {
    guard shouldInstrument else { return try await session.data(for: request) }
    let instrumentation = NetworkTaskInstrumentation(
      request: request,
      propagation: propagation
    )
    let prepared = instrumentation.prepared(request)
    do {
      let result = try await TraceTaskContext.$current.withValue(
        instrumentation.span.context
      ) {
        try await session.data(
          for: prepared,
          delegate: instrumentation
        )
      }
      instrumentation.finish(response: result.1)
      return result
    } catch {
      instrumentation.finish(error: error)
      throw error
    }
  }

  public func data(from url: URL) async throws -> (Data, URLResponse) {
    try await data(for: URLRequest(url: url))
  }

  public func upload(
    for request: URLRequest,
    from bodyData: Data
  ) async throws -> (Data, URLResponse) {
    guard shouldInstrument else {
      return try await session.upload(for: request, from: bodyData)
    }
    let instrumentation = NetworkTaskInstrumentation(
      request: request,
      propagation: propagation
    )
    let prepared = instrumentation.prepared(request)
    do {
      let result = try await TraceTaskContext.$current.withValue(
        instrumentation.span.context
      ) {
        try await session.upload(
          for: prepared,
          from: bodyData,
          delegate: instrumentation
        )
      }
      instrumentation.finish(response: result.1)
      return result
    } catch {
      instrumentation.finish(error: error)
      throw error
    }
  }

  public func download(
    for request: URLRequest
  ) async throws -> (URL, URLResponse) {
    guard shouldInstrument else {
      return try await session.download(for: request)
    }
    let instrumentation = NetworkTaskInstrumentation(
      request: request,
      propagation: propagation
    )
    let prepared = instrumentation.prepared(request)
    do {
      let result = try await TraceTaskContext.$current.withValue(
        instrumentation.span.context
      ) {
        try await session.download(
          for: prepared,
          delegate: instrumentation
        )
      }
      instrumentation.finish(response: result.1)
      return result
    } catch {
      instrumentation.finish(error: error)
      throw error
    }
  }

  private var shouldInstrument: Bool {
    ChillTraceRuntime.isProviderConfigured
      || !propagation.trustedOrigins.isEmpty
  }
}

extension URLSession {
  /// Opts an existing session into automatic task instrumentation.
  public func chill(
    _ propagation: ChillNetworkPropagationPolicy = .none
  ) -> ChillURLSession {
    ChillURLSession(session: self, propagation: propagation)
  }
}

private final class NetworkTaskInstrumentation: NSObject,
  URLSessionTaskDelegate,
  @unchecked Sendable
{
  let span: ChillTraceSpan
  private let propagation: ChillNetworkPropagationPolicy
  private let startedAt = DispatchTime.now().uptimeNanoseconds
  private let state = NSLock()
  private var redirectCount: Int64 = 0

  init(
    request: URLRequest,
    propagation: ChillNetworkPropagationPolicy
  ) {
    self.propagation = propagation
    let method = Self.normalizedMethod(request.httpMethod)
    var attributes: [String: ChillTraceAttributeValue] = [
      "http.request.method": .string(method)
    ]
    if let origin = ChillNetworkOrigin(destination: request.url) {
      attributes["url.scheme"] = .string(origin.scheme)
      attributes["server.address"] = .string(origin.host)
      if let port = origin.port {
        attributes["server.port"] = .integer(Int64(port))
      }
    }
    span = ChillTraceRuntime.startSpan(
      name: method,
      kind: .client,
      attributes: attributes
    )
  }

  private static func normalizedMethod(_ rawValue: String?) -> String {
    let method = rawValue?.uppercased() ?? "GET"
    switch method {
    case "CONNECT", "DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST",
      "PUT", "TRACE":
      return method
    default:
      return "_OTHER"
    }
  }

  func prepared(_ original: URLRequest) -> URLRequest {
    NetworkHeaderPropagation.prepare(
      original,
      context: span.context,
      policy: propagation
    )
  }

  func urlSession(
    _ session: URLSession,
    task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse,
    newRequest request: URLRequest,
    completionHandler: @escaping @Sendable (URLRequest?) -> Void
  ) {
    state.withLock { redirectCount += 1 }
    completionHandler(prepared(request))
  }

  func finish(response: URLResponse) {
    var attributes = terminalAttributes()
    var status: ChillTraceSpanStatus = .unset
    if let response = response as? HTTPURLResponse {
      attributes["http.response.status_code"] = .integer(
        Int64(response.statusCode)
      )
      if response.statusCode >= 400 {
        attributes["error.type"] = .string(String(response.statusCode))
        status = .error
      }
    }
    span.end(
      status: status,
      attributes: attributes
    )
  }

  func finish(error: any Error) {
    var attributes = terminalAttributes()
    let status: ChillTraceSpanStatus
    if let urlError = error as? URLError {
      attributes["error.type"] = .string("URLError.\(urlError.code.rawValue)")
      status = urlError.code == .cancelled ? .unset : .error
    } else if error is CancellationError {
      attributes["error.type"] = .string("CancellationError")
      status = .unset
    } else {
      attributes["error.type"] = .string("Error")
      status = .error
    }
    span.end(status: status, attributes: attributes)
  }

  private func terminalAttributes() -> [String: ChillTraceAttributeValue] {
    let endedAt = DispatchTime.now().uptimeNanoseconds
    let duration = endedAt >= startedAt ? endedAt - startedAt : 0
    let redirects = state.withLock { redirectCount }
    return [
      "chill.duration.nano": .integer(Int64(clamping: duration)),
      "http.redirect.count": .integer(redirects),
    ]
  }
}

package enum NetworkHeaderPropagation {
  package static func prepare(
    _ original: URLRequest,
    context: TraceContext,
    policy: ChillNetworkPropagationPolicy
  ) -> URLRequest {
    var request = original
    request.setValue(nil, forHTTPHeaderField: "traceparent")
    request.setValue(nil, forHTTPHeaderField: "tracestate")
    request.setValue(nil, forHTTPHeaderField: "baggage")
    guard policy.trusts(request.url) else { return request }
    request.setValue(
      W3CTraceContext.traceParent(for: context),
      forHTTPHeaderField: "traceparent"
    )
    if let traceState = context.traceState {
      request.setValue(traceState, forHTTPHeaderField: "tracestate")
    }
    if let baggage = policy.baggageHeader {
      request.setValue(baggage, forHTTPHeaderField: "baggage")
    }
    return request
  }
}
