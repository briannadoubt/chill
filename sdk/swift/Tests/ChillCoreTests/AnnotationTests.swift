import Testing

@testable import ChillCore

@Test
func typedAnnotationsResolveOutermostFirst() throws {
  let tier = try AnnotationKey<String>("account.tier")
  let cohorts = try AnnotationKey<[String]>("experiment.cohorts")
  let root = try AnnotationContext().addingScope(
    id: AnnotationScopeID("root"),
    declarations: [
      try AnnotationDeclaration(tier, value: "enterprise"),
      try AnnotationDeclaration(cohorts, value: ["search_a", "checkout_b"]),
    ]
  )
  let page = try root.addingScope(
    id: AnnotationScopeID("page.cat"),
    declarations: [
      try AnnotationDeclaration("cat.id", value: "cat_123"),
      try AnnotationDeclaration(tier, value: "free"),
    ]
  )
  let action = try page.addingScope(
    id: AnnotationScopeID("action.adopt"),
    declarations: [
      try AnnotationDeclaration("cat.id", value: "cat_999"),
      try AnnotationDeclaration(tier, value: "enterprise"),
    ]
  )

  #expect(action[tier] == "enterprise")
  #expect(action[cohorts] == ["search_a", "checkout_b"])
  #expect(action.snapshot.values[try AnnotationName("cat.id")] == .string("cat_123"))
  #expect(action.snapshot.collisions.count == 3)
  #expect(action.snapshot.collisions[0].identical == false)
  #expect(action.snapshot.collisions[2].identical == true)
  let rootID = try AnnotationScopeID("root")
  #expect(action.snapshot.origins[tier.name]?.scopeID == rootID)
}

@Test
func annotationContextsAreImmutableAndSiblingsAreIsolated() throws {
  let root = try AnnotationContext().addingScope(
    id: AnnotationScopeID("root"),
    declarations: [try AnnotationDeclaration("tenant.id", value: "tenant-a")]
  )
  let left = try root.addingScope(
    id: AnnotationScopeID("left"),
    declarations: [try AnnotationDeclaration("cat.id", value: "cat-left")]
  )
  let right = try root.addingScope(
    id: AnnotationScopeID("right"),
    declarations: [try AnnotationDeclaration("cat.id", value: "cat-right")]
  )
  let cat = try AnnotationKey<String>("cat.id")

  #expect(root[cat] == nil)
  #expect(left[cat] == "cat-left")
  #expect(right[cat] == "cat-right")
}

@Test
func annotationBoundsAndInvalidValuesFailClosed() throws {
  #expect(throws: ContractError.self) {
    _ = try AnnotationName("Account.Tier")
  }
  #expect(throws: ContractError.self) {
    _ = try AnnotationDeclaration("metric.value", value: Double.nan)
  }
  #expect(throws: ContractError.self) {
    _ = try AnnotationDeclaration(
      "too.long",
      value: String(repeating: "x", count: 4_097)
    )
  }
  #expect(throws: ContractError.self) {
    _ = try AnnotationDeclaration(
      "too.many",
      value: Array(repeating: true, count: 65)
    )
  }

  let declarations = try (0...128).map {
    try AnnotationDeclaration("key\($0)", value: $0)
  }
  #expect(throws: ContractError.self) {
    _ = try AnnotationContext().addingScope(
      id: AnnotationScopeID("root"),
      declarations: declarations
    )
  }
}

@Test
func duplicateScopeIdentityIsRejected() throws {
  let scope = try AnnotationScopeID("root")
  let root = try AnnotationContext().addingScope(id: scope, declarations: [])
  #expect(throws: ContractError.self) {
    _ = try root.addingScope(id: scope, declarations: [])
  }
}
