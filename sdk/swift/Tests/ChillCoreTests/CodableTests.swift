import Foundation
import Testing

@testable import ChillCore

@Test
func stringBackedContractValuesUseSingleJSONStrings() throws {
  let value = try PageInstanceID("page-123")
  let encoded = try JSONEncoder().encode(value)

  #expect(String(decoding: encoded, as: UTF8.self) == "\"page-123\"")
  #expect(try JSONDecoder().decode(PageInstanceID.self, from: encoded) == value)
}

@Test
func annotationValuesUseTheCanonicalUntaggedJSONShape() throws {
  let values: [(AnnotationValue, String)] = [
    (.string("enterprise"), "\"enterprise\""),
    (.boolean(true), "true"),
    (.integer(42), "42"),
    (.double(2.5), "2.5"),
    (.strings(["a", "b"]), "[\"a\",\"b\"]"),
    (.booleans([true, false]), "[true,false]"),
    (.integers([1, 2]), "[1,2]"),
    (.doubles([1.5, 2.5]), "[1.5,2.5]"),
  ]

  for (value, expected) in values {
    let encoded = try JSONEncoder().encode(value)
    #expect(String(decoding: encoded, as: UTF8.self) == expected)
    #expect(try JSONDecoder().decode(AnnotationValue.self, from: encoded) == value)
  }
}
