import CChillCompression
import Foundation

package struct OTLPExportRequest: Sendable {
  package let recordIDs: Set<String>
  package let body: Data
}

package enum OTLPExportDisposition: Equatable, Sendable {
  case acknowledged(Set<String>)
  case retry(after: TimeInterval?)
  case blocked(statusCode: Int?)
}

package protocol OTLPExportTransport: Sendable {
  func export(_ request: OTLPExportRequest) async -> OTLPExportDisposition
}

package final class OTLPHTTPTransport: OTLPExportTransport,
  @unchecked Sendable
{
  private let configuration: ChillOTLPConfiguration
  private let session: URLSession

  package init(
    configuration: ChillOTLPConfiguration,
    session: URLSession? = nil
  ) {
    self.configuration = configuration
    if let session {
      self.session = session
    } else {
      let sessionConfiguration = URLSessionConfiguration.ephemeral
      sessionConfiguration.waitsForConnectivity = true
      sessionConfiguration.timeoutIntervalForRequest = 30
      sessionConfiguration.timeoutIntervalForResource = 60
      self.session = URLSession(configuration: sessionConfiguration)
    }
  }

  package func export(
    _ export: OTLPExportRequest
  ) async -> OTLPExportDisposition {
    var request = URLRequest(url: configuration.endpoint)
    request.httpMethod = "POST"
    request.setValue(
      "application/x-protobuf",
      forHTTPHeaderField: "Content-Type"
    )
    request.setValue("application/x-protobuf", forHTTPHeaderField: "Accept")
    for (key, value) in configuration.headers {
      request.setValue(value, forHTTPHeaderField: key)
    }
    if let compressed = GzipCompression.compress(export.body),
      compressed.count < export.body.count
    {
      request.httpBody = compressed
      request.setValue("gzip", forHTTPHeaderField: "Content-Encoding")
    } else {
      request.httpBody = export.body
    }

    do {
      let (_, response) = try await session.data(for: request)
      guard let response = response as? HTTPURLResponse else {
        return .retry(after: nil)
      }
      switch response.statusCode {
      case 200..<300:
        return .acknowledged(export.recordIDs)
      case 408, 429, 500..<600:
        return .retry(after: Self.retryAfter(response))
      default:
        // Keep the batch durable but pause automatic hot-loop retries for
        // authentication, configuration, and schema errors.
        return .blocked(statusCode: response.statusCode)
      }
    } catch let error as URLError {
      if error.code == .cancelled {
        return .blocked(statusCode: nil)
      }
      return .retry(after: nil)
    } catch is CancellationError {
      return .blocked(statusCode: nil)
    } catch {
      return .retry(after: nil)
    }
  }

  private static func retryAfter(_ response: HTTPURLResponse) -> TimeInterval? {
    guard let value = response.value(forHTTPHeaderField: "Retry-After"),
      let seconds = TimeInterval(value),
      seconds.isFinite,
      (0...3_600).contains(seconds)
    else { return nil }
    return seconds
  }
}

package enum GzipCompression {
  package static func compress(_ data: Data) -> Data? {
    let capacity = chill_gzip_bound(data.count)
    guard capacity > 0 else { return nil }
    var destination = Data(count: capacity)
    var outputSize = capacity
    let status = data.withUnsafeBytes { sourceBuffer in
      destination.withUnsafeMutableBytes { destinationBuffer in
        chill_gzip_compress(
          sourceBuffer.bindMemory(to: UInt8.self).baseAddress,
          data.count,
          destinationBuffer.bindMemory(to: UInt8.self).baseAddress,
          &outputSize
        )
      }
    }
    guard status == 0, outputSize <= destination.count else { return nil }
    destination.removeSubrange(outputSize..<destination.count)
    return destination
  }

  package static func decompress(
    _ data: Data,
    expectedByteCount: Int
  ) -> Data? {
    guard expectedByteCount >= 0 else { return nil }
    var destination = Data(count: expectedByteCount)
    var outputSize = expectedByteCount
    let status = data.withUnsafeBytes { sourceBuffer in
      destination.withUnsafeMutableBytes { destinationBuffer in
        chill_gzip_decompress(
          sourceBuffer.bindMemory(to: UInt8.self).baseAddress,
          data.count,
          destinationBuffer.bindMemory(to: UInt8.self).baseAddress,
          &outputSize
        )
      }
    }
    guard status == 0, outputSize == expectedByteCount else { return nil }
    return destination
  }
}
