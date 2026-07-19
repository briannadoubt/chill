public enum PageRelation: String, Codable, Sendable {
  case root
  case push
  case tab
  case split
  case sheet
  case popover
  case overlay
  case cover
}

public enum PageExposure: String, Codable, Sendable {
  case foreground
  case visible
  case occluded
  case retained
}

public struct PagePath: Equatable, Sendable {
  public let surfaceID: SurfaceID
  public let instanceID: PageInstanceID
  public let instanceIDs: [PageInstanceID]
  public let segments: [PageSegment]
  public let relation: PageRelation
  public let exposure: PageExposure
  public let focused: Bool

  public init(
    surfaceID: SurfaceID,
    instanceID: PageInstanceID,
    instanceIDs: [PageInstanceID],
    segments: [PageSegment],
    relation: PageRelation,
    exposure: PageExposure,
    focused: Bool
  ) throws {
    guard !instanceIDs.isEmpty else { throw ContractError.emptyPagePath }
    guard instanceIDs.count <= 32 else {
      throw ContractError.pagePathLimitExceeded
    }
    guard instanceIDs.count == segments.count,
      instanceIDs.last == instanceID
    else {
      throw ContractError.pagePathLengthMismatch
    }
    self.surfaceID = surfaceID
    self.instanceID = instanceID
    self.instanceIDs = instanceIDs
    self.segments = segments
    self.relation = relation
    self.exposure = exposure
    self.focused = focused
  }
}

public struct ElementIdentity: Equatable, Sendable {
  public let surfaceID: SurfaceID
  public let instanceID: ElementInstanceID
  public let name: SemanticName
  public let role: SemanticName

  public init(
    surfaceID: SurfaceID,
    instanceID: ElementInstanceID,
    name: SemanticName,
    role: SemanticName
  ) {
    self.surfaceID = surfaceID
    self.instanceID = instanceID
    self.name = name
    self.role = role
  }
}
