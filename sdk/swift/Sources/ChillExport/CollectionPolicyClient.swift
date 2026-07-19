import ChillCore
import Foundation

public enum ChillCollectionPolicyError: Error, Equatable, Sendable {
  case invalidEndpoint
  case invalidHeader
  case invalidResponse
  case unauthorized
  case unavailable
  case responseTooLarge
}

public final class ChillCollectionPolicyClient: @unchecked Sendable {
  private let endpoint: URL
  private let headers: [String: String]
  private let session: URLSession

  public convenience init(
    endpoint: URL,
    headers: [String: String] = [:],
    allowInsecureLocalhost: Bool = false
  ) throws {
    try self.init(
      endpoint: endpoint,
      headers: headers,
      allowInsecureLocalhost: allowInsecureLocalhost,
      session: nil
    )
  }

  package init(
    endpoint: URL,
    headers: [String: String],
    allowInsecureLocalhost: Bool,
    session: URLSession?
  ) throws {
    guard endpoint.user == nil,
      endpoint.password == nil,
      endpoint.query == nil,
      endpoint.fragment == nil,
      endpoint.path == "/v1/chill/collection-state",
      endpoint.scheme?.lowercased() == "https"
        || (allowInsecureLocalhost
          && endpoint.scheme?.lowercased() == "http"
          && ["localhost", "127.0.0.1", "::1"].contains(
            endpoint.host?.lowercased()
          ))
    else {
      throw ChillCollectionPolicyError.invalidEndpoint
    }
    guard headers.count <= 8,
      headers.allSatisfy({ key, value in
        !key.isEmpty && key.utf8.count <= 128 && value.utf8.count <= 4_096
          && !value.utf8.contains(10) && !value.utf8.contains(13)
          && !["accept", "content-length", "content-type"].contains(
            key.lowercased()
          )
      })
    else {
      throw ChillCollectionPolicyError.invalidHeader
    }
    self.endpoint = endpoint
    self.headers = headers
    if let session {
      self.session = session
    } else {
      let configuration = URLSessionConfiguration.ephemeral
      configuration.waitsForConnectivity = true
      configuration.timeoutIntervalForRequest = 15
      configuration.timeoutIntervalForResource = 30
      self.session = URLSession(configuration: configuration)
    }
  }

  public func fetch() async throws -> RemoteCollectionState {
    var request = URLRequest(url: endpoint)
    request.httpMethod = "GET"
    request.setValue("application/json", forHTTPHeaderField: "Accept")
    for (key, value) in headers {
      request.setValue(value, forHTTPHeaderField: key)
    }
    let (body, response): (Data, URLResponse)
    do {
      (body, response) = try await session.data(for: request)
    } catch is CancellationError {
      throw CancellationError()
    } catch {
      throw ChillCollectionPolicyError.unavailable
    }
    guard let http = response as? HTTPURLResponse else {
      throw ChillCollectionPolicyError.invalidResponse
    }
    switch http.statusCode {
    case 200:
      break
    case 401, 403:
      throw ChillCollectionPolicyError.unauthorized
    case 408, 429, 500..<600:
      throw ChillCollectionPolicyError.unavailable
    default:
      throw ChillCollectionPolicyError.invalidResponse
    }
    guard body.count <= 64 * 1_024 else {
      throw ChillCollectionPolicyError.responseTooLarge
    }
    do {
      return try JSONDecoder().decode(RemoteCollectionState.self, from: body)
    } catch {
      throw ChillCollectionPolicyError.invalidResponse
    }
  }
}

public actor ChillCollectionPolicySynchronizer {
  private let runtime: ChillRuntime
  private let client: ChillCollectionPolicyClient

  public init(runtime: ChillRuntime, client: ChillCollectionPolicyClient) {
    self.runtime = runtime
    self.client = client
  }

  @discardableResult
  public func refresh() async throws -> CollectionStateApplyResult {
    runtime.applyCollectionState(try await client.fetch())
  }
}
