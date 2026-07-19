import Testing

@testable import ChillCore

@Test
func consentClassesAreIndependentAndDefaultClosed() throws {
  let policy = try PrivacyPolicy(
    version: "privacy-v1",
    analytics: .granted,
    diagnostic: .denied,
    replay: .unknown
  )

  #expect(policy.allows(.essential))
  #expect(policy.allows(.analytics))
  #expect(!policy.allows(.diagnostic))
  #expect(!policy.allows(.replay))
}

@Test
func sensitiveSourceDataIsOmittedOrMaskedBeforeBuffering() throws {
  let replay = try PrivacyPolicy(
    version: "privacy-v1",
    analytics: .granted,
    replay: .granted,
    explicitlyAllowed: [.headers, .secureInput, .clipboard]
  )

  #expect(replay.sourceDisposition(for: .uiText, captureClass: .replay) == .mask)
  #expect(replay.sourceDisposition(for: .pixels, captureClass: .replay) == .mask)
  #expect(replay.sourceDisposition(for: .headers, captureClass: .analytics) == .allow)
  #expect(replay.sourceDisposition(for: .body, captureClass: .analytics) == .omit)
  #expect(replay.sourceDisposition(for: .secureInput, captureClass: .replay) == .mask)
  #expect(replay.sourceDisposition(for: .clipboard, captureClass: .analytics) == .mask)
  #expect(replay.sourceDisposition(for: .secret, captureClass: .analytics) == .omit)
  #expect(replay.sourceDisposition(for: .credential, captureClass: .analytics) == .omit)
}

@Test
func customAnnotationsRequireAnExactSafeAllowlistEntry() throws {
  let policy = try PrivacyPolicy(
    version: "privacy-v1",
    analytics: .granted,
    annotationAllowlist: [
      "account.tier": .internalData,
      "cat.id": .pseudonymousIdentifier,
      "unsafe.note": .sensitiveData,
    ]
  )
  let declared = try AnnotationContext().addingScope(
    id: AnnotationScopeID("privacy-test"),
    declarations: [
      AnnotationDeclaration("account.tier", value: "internal"),
      AnnotationDeclaration("cat.id", value: "cat-42"),
      AnnotationDeclaration("unsafe.note", value: "never serialize"),
      AnnotationDeclaration("unknown.value", value: "default denied"),
    ]
  ).snapshot

  let result = policy.classifyAnnotations(declared)

  #expect(result.omitted == 2)
  #expect(result.snapshot.values.count == 2)
  #expect(
    result.snapshot.classifications[try AnnotationName("account.tier")]
      == .internalData
  )
  #expect(
    result.snapshot.classifications[try AnnotationName("cat.id")]
      == .pseudonymousIdentifier
  )
  #expect(result.snapshot.values[try AnnotationName("unsafe.note")] == nil)
  #expect(result.snapshot.values[try AnnotationName("unknown.value")] == nil)
}

@Test
func samplingMatchesTheCrossPlatformGoldenAlgorithm() throws {
  let sampler = try DeterministicSampler(salt: "test-salt-v1")
  let behavior = try SamplingRate(numerator: 1, denominator: 2)
  let replay = try SamplingRate(numerator: 1, denominator: 4)

  let user2Behavior = sampler.decide(
    stream: .behavior,
    stableKey: "user-2",
    rate: behavior
  )
  let user2Replay = sampler.decide(
    stream: .replay,
    stableKey: "user-2",
    rate: replay
  )
  let user0Behavior = sampler.decide(
    stream: .behavior,
    stableKey: "user-0",
    rate: behavior
  )

  #expect(user2Behavior == SamplingDecision(kept: true, digestPrefix: "6e18324b28d8b1e8"))
  #expect(user2Replay == SamplingDecision(kept: false, digestPrefix: "aba66fd86a055fc0"))
  #expect(user0Behavior == SamplingDecision(kept: false, digestPrefix: "a7f598b903ffef97"))
}

@Test
func samplingRatesAreExactAtTheBoundaries() throws {
  let sampler = try DeterministicSampler(salt: "salt")
  #expect(sampler.decide(stream: .behavior, stableKey: "key", rate: .all).kept)
  #expect(!sampler.decide(stream: .behavior, stableKey: "key", rate: .none).kept)
  #expect(throws: ContractError.self) {
    _ = try SamplingRate(numerator: 2, denominator: 1)
  }
  #expect(throws: ContractError.self) {
    _ = try SamplingRate(numerator: 0, denominator: 0)
  }
}
