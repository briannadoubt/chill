import Foundation
import Testing

@testable import ChillCore

@Test
func semanticNamesAndPageSegmentsAreBounded() throws {
  #expect(try SemanticName("cat.adopt") == SemanticName("cat.adopt"))
  #expect(try SemanticName("cat_adopt") == SemanticName("cat_adopt"))
  #expect(throws: ContractError.self) { _ = try SemanticName("Cat Adopt") }
  #expect(throws: ContractError.self) { _ = try SemanticName("cat..adopt") }
  #expect(throws: ContractError.self) {
    _ = try PageSegment(String(repeating: "a", count: 81))
  }
}

@Test
func generatedRecordIDsAreRFC9562UUIDv7Values() {
  let uuid = UUIDv7.generate().uuidString.lowercased()
  let characters = Array(uuid)

  #expect(characters[14] == "7")
  #expect(["8", "9", "a", "b"].contains(characters[19]))
}

@Test
func pagePathsPreserveStructuredSegmentsAndIdentity() throws {
  let root = try PageInstanceID("page-root")
  let cats = try PageInstanceID("page-cats")
  let cat = try PageInstanceID("page-cat")
  let path = try PagePath(
    surfaceID: SurfaceID("surface-main"),
    instanceID: cat,
    instanceIDs: [root, cats, cat],
    segments: [PageSegment("app"), PageSegment("cats"), PageSegment("cat")],
    relation: .push,
    exposure: .foreground,
    focused: true
  )

  #expect(path.segments.map(\.rawValue) == ["app", "cats", "cat"])
  #expect(path.instanceIDs == [root, cats, cat])
  #expect(path.focused)
}

@Test
func invalidPagePathsAreRejected() throws {
  #expect(throws: ContractError.self) {
    _ = try PagePath(
      surfaceID: SurfaceID("surface-main"),
      instanceID: PageInstanceID("page-root"),
      instanceIDs: [PageInstanceID("page-root")],
      segments: [],
      relation: .root,
      exposure: .foreground,
      focused: true
    )
  }
  #expect(throws: ContractError.self) {
    let ids = try (0..<33).map { try PageInstanceID("page-\($0)") }
    let segments = try (0..<33).map { try PageSegment("page\($0)") }
    _ = try PagePath(
      surfaceID: SurfaceID("surface-main"),
      instanceID: ids.last!,
      instanceIDs: ids,
      segments: segments,
      relation: .push,
      exposure: .foreground,
      focused: true
    )
  }
}
