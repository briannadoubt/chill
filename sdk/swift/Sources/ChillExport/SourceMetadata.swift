import Foundation

package struct ChillSourceMetadata: Equatable, Sendable {
  package let platform: String
  package let installationID: String
  package let processID: String
  package let appBuild: String?

  package static func persistent(
    in directory: URL,
    fileManager: FileManager = .default,
    bundle: Bundle = .main
  ) throws -> Self {
    try fileManager.createDirectory(
      at: directory,
      withIntermediateDirectories: true
    )
    let identityURL = directory.appendingPathComponent(
      ".chill-installation-id",
      isDirectory: false
    )
    let installationID: String
    if let existing = try? String(contentsOf: identityURL, encoding: .utf8)
      .trimmingCharacters(in: .whitespacesAndNewlines),
      isUUIDv4(existing)
    {
      installationID = existing
    } else {
      installationID = UUID().uuidString.lowercased()
      var options: Data.WritingOptions = [.atomic]
      #if os(iOS) || os(tvOS) || os(watchOS) || os(visionOS)
        options.insert(.completeFileProtectionUntilFirstUserAuthentication)
      #endif
      try Data("\(installationID)\n".utf8).write(
        to: identityURL,
        options: options
      )
      try? fileManager.setAttributes(
        [.posixPermissions: 0o600],
        ofItemAtPath: identityURL.path
      )
    }
    let appBuild =
      bundle.object(
        forInfoDictionaryKey: "CFBundleVersion"
      ) as? String
    return Self(
      platform: "apple",
      installationID: installationID,
      processID: UUID().uuidString.lowercased(),
      appBuild: appBuild.flatMap { $0.isEmpty ? nil : $0 }
    )
  }

  package init(
    platform: String,
    installationID: String,
    processID: String,
    appBuild: String?
  ) {
    self.platform = platform
    self.installationID = installationID
    self.processID = processID
    self.appBuild = appBuild
  }

  private static func isUUIDv4(_ value: String) -> Bool {
    guard value == value.lowercased(),
      value.utf8.count == 36,
      UUID(uuidString: value) != nil
    else { return false }
    let characters = Array(value.utf8)
    return characters[14] == 52
      && [56, 57, 97, 98].contains(characters[19])
  }
}
