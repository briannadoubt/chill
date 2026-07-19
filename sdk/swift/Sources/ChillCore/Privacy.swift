public enum CaptureClass: String, CaseIterable, Codable, Sendable {
  case essential
  case analytics
  case diagnostic
  case replay
}

public enum ConsentState: String, Codable, Sendable {
  case unknown
  case granted
  case denied
  case notRequired = "not_required"
}

public enum SensitiveDataCategory: String, CaseIterable, Codable, Sendable {
  case deviceIdentifier
  case deviceMetadata
  case uiText
  case formValue
  case accessibilityText
  case secureInput
  case clipboard
  case pixels
  case url
  case query
  case headers
  case body
  case errorDescription
  case arbitraryPayload
  case customAnnotation
  case secret
  case credential
}

public enum SourceDisposition: String, Codable, Sendable {
  case omit
  case mask
  case allow
}

public enum DataClassification: String, CaseIterable, Codable, Sendable {
  case publicData = "public"
  case internalData = "internal"
  case pseudonymousIdentifier = "pseudonymous_identifier"
  case personalData = "personal_data"
  case sensitiveData = "sensitive_data"
  case secret
  case credential

  @inlinable
  public var isAnnotationEligible: Bool {
    switch self {
    case .publicData, .internalData, .pseudonymousIdentifier, .personalData:
      true
    case .sensitiveData, .secret, .credential:
      false
    }
  }
}

public struct PrivacyPolicy: Equatable, Sendable {
  public let version: String
  private let consent: [CaptureClass: ConsentState]
  private let explicitlyAllowed: Set<SensitiveDataCategory>
  private let annotationAllowlist: [AnnotationName: DataClassification]

  public init(
    version: String,
    essential: ConsentState = .granted,
    analytics: ConsentState = .unknown,
    diagnostic: ConsentState = .unknown,
    replay: ConsentState = .unknown,
    explicitlyAllowed: Set<SensitiveDataCategory> = [],
    annotationAllowlist: [String: DataClassification] = [:]
  ) throws {
    guard version.utf8.count <= 64 else {
      throw ContractError.invalidPrivacyPolicyVersion
    }
    self.version = try _validatedIdentifier(
      version,
      type: "PrivacyPolicy.version"
    )
    consent = [
      .essential: essential,
      .analytics: analytics,
      .diagnostic: diagnostic,
      .replay: replay,
    ]
    self.explicitlyAllowed = explicitlyAllowed.subtracting([
      .secureInput,
      .clipboard,
      .secret,
      .credential,
    ])
    var validatedAnnotations: [AnnotationName: DataClassification] = [:]
    validatedAnnotations.reserveCapacity(annotationAllowlist.count)
    for (name, classification) in annotationAllowlist {
      validatedAnnotations[try AnnotationName(name)] = classification
    }
    self.annotationAllowlist = validatedAnnotations
  }

  public func consentState(for captureClass: CaptureClass) -> ConsentState {
    consent[captureClass] ?? .denied
  }

  @inlinable
  public func allows(_ captureClass: CaptureClass) -> Bool {
    let state = consentState(for: captureClass)
    return state == .granted || state == .notRequired
  }

  public func sourceDisposition(
    for category: SensitiveDataCategory,
    captureClass: CaptureClass
  ) -> SourceDisposition {
    guard allows(captureClass) else { return .omit }
    if category == .secureInput || category == .clipboard {
      return .mask
    }
    if category == .secret || category == .credential {
      return .omit
    }
    if explicitlyAllowed.contains(category) {
      return .allow
    }
    if captureClass == .replay,
      [.uiText, .formValue, .accessibilityText, .pixels].contains(category)
    {
      return .mask
    }
    return .omit
  }

  public func annotationClassification(
    for name: AnnotationName
  ) -> DataClassification? {
    annotationAllowlist[name]
  }

  package func classifyAnnotations(
    _ snapshot: AnnotationSnapshot
  ) -> (snapshot: AnnotationSnapshot, omitted: UInt32) {
    guard !snapshot.values.isEmpty else { return (snapshot, 0) }
    var values: [AnnotationName: AnnotationValue] = [:]
    var origins: [AnnotationName: AnnotationOrigin] = [:]
    var classifications: [AnnotationName: DataClassification] = [:]
    values.reserveCapacity(snapshot.values.count)
    origins.reserveCapacity(snapshot.origins.count)
    classifications.reserveCapacity(snapshot.values.count)
    for (name, value) in snapshot.values {
      guard let classification = annotationAllowlist[name],
        classification.isAnnotationEligible
      else { continue }
      values[name] = value
      origins[name] = snapshot.origins[name]
      classifications[name] = classification
    }
    let collisions = snapshot.collisions.filter { values[$0.name] != nil }
    return (
      AnnotationSnapshot(
        values: values,
        origins: origins,
        collisions: collisions,
        classifications: classifications
      ),
      UInt32(clamping: snapshot.values.count - values.count)
    )
  }
}
