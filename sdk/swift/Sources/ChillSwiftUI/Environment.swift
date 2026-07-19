import ChillCore
import Observation
import SwiftUI

struct SwiftUIPageAncestry: Equatable, Sendable {
  let path: PagePath
}

@MainActor
@Observable
final class PageSurfaceCoordinator {
  private struct Entry: Equatable {
    let parentInstanceID: PageInstanceID?
    let relation: PageRelation
    let declaredExposure: PageExposure
    let declaredFocused: Bool
    let order: UInt64
    let mounted: Bool
  }

  private var entries: [PageInstanceID: Entry] = [:]
  private var nextOrder: UInt64 = 0

  func register(
    path: PagePath,
    parentInstanceID: PageInstanceID?,
    declaredExposure: PageExposure,
    declaredFocused: Bool
  ) {
    let existing = entries[path.instanceID]
    if existing == nil {
      nextOrder &+= 1
    }
    let updated = Entry(
      parentInstanceID: parentInstanceID,
      relation: path.relation,
      declaredExposure: declaredExposure,
      declaredFocused: declaredFocused,
      order: existing?.order ?? nextOrder,
      mounted: true
    )
    guard existing != updated else { return }
    entries[path.instanceID] = updated
  }

  func setMounted(_ mounted: Bool, for instanceID: PageInstanceID) {
    guard let entry = entries[instanceID], entry.mounted != mounted else {
      return
    }
    entries[instanceID] = Entry(
      parentInstanceID: entry.parentInstanceID,
      relation: entry.relation,
      declaredExposure: entry.declaredExposure,
      declaredFocused: entry.declaredFocused,
      order: entry.order,
      mounted: mounted
    )
  }

  func remove(_ instanceID: PageInstanceID) {
    entries.removeValue(forKey: instanceID)
  }

  func resolve(_ path: PagePath) -> PagePath {
    guard entries[path.instanceID] != nil else { return path }
    let exposure = effectiveExposure(for: path.instanceID)
    let focused =
      exposure == .foreground && focusedInstanceID == path.instanceID
    return
      (try? PagePath(
        surfaceID: path.surfaceID,
        instanceID: path.instanceID,
        instanceIDs: path.instanceIDs,
        segments: path.segments,
        relation: path.relation,
        exposure: exposure,
        focused: focused
      )) ?? path
  }

  private var focusedInstanceID: PageInstanceID? {
    var candidates: [(PageInstanceID, Int, UInt64)] = []
    for (instanceID, entry) in entries {
      guard entry.mounted,
        entry.declaredFocused,
        effectiveExposure(for: instanceID) == .foreground
      else {
        continue
      }
      candidates.append((instanceID, depth(of: instanceID), entry.order))
    }
    return candidates.max { left, right in
      if left.1 != right.1 { return left.1 < right.1 }
      return left.2 < right.2
    }?.0
  }

  private func effectiveExposure(for instanceID: PageInstanceID) -> PageExposure {
    guard let entry = entries[instanceID] else { return .retained }
    var activeChildRelations: [PageRelation] = []
    for (childID, child) in entries {
      if child.parentInstanceID == instanceID,
        branchIsActive(at: childID)
      {
        activeChildRelations.append(child.relation)
      }
    }
    if activeChildRelations.contains(.cover) {
      return .occluded
    }
    if activeChildRelations.contains(where: {
      $0 == .push || $0 == .tab
    }) {
      return .retained
    }
    if activeChildRelations.contains(where: {
      $0 == .sheet || $0 == .popover || $0 == .overlay || $0 == .split
    }) {
      return .visible
    }
    return entry.mounted ? entry.declaredExposure : .retained
  }

  private func branchIsActive(at instanceID: PageInstanceID) -> Bool {
    guard let entry = entries[instanceID] else { return false }
    if entry.mounted, entry.declaredExposure != .retained { return true }
    for (childID, child) in entries {
      if child.parentInstanceID == instanceID,
        branchIsActive(at: childID)
      {
        return true
      }
    }
    return false
  }

  private func depth(of instanceID: PageInstanceID) -> Int {
    var depth = 0
    var cursor = entries[instanceID]?.parentInstanceID
    var visited: Set<PageInstanceID> = [instanceID]
    while let current = cursor, visited.insert(current).inserted {
      depth += 1
      cursor = entries[current]?.parentInstanceID
    }
    return depth
  }
}

private struct ChillAnnotationContextKey: EnvironmentKey {
  static let defaultValue = AnnotationContext()
}

private struct ChillPageAncestryKey: EnvironmentKey {
  static let defaultValue: SwiftUIPageAncestry? = nil
}

private struct ChillCapturePolicyKey: EnvironmentKey {
  static let defaultValue = ViewCapturePolicy.standard
}

private struct ChillPageSurfaceCoordinatorKey: EnvironmentKey {
  static let defaultValue: PageSurfaceCoordinator? = nil
}

extension EnvironmentValues {
  var chillAnnotationContext: AnnotationContext {
    get { self[ChillAnnotationContextKey.self] }
    set { self[ChillAnnotationContextKey.self] = newValue }
  }

  var chillPageAncestry: SwiftUIPageAncestry? {
    get { self[ChillPageAncestryKey.self] }
    set { self[ChillPageAncestryKey.self] = newValue }
  }

  var chillCapturePolicy: ViewCapturePolicy {
    get { self[ChillCapturePolicyKey.self] }
    set { self[ChillCapturePolicyKey.self] = newValue }
  }

  var chillPageSurfaceCoordinator: PageSurfaceCoordinator? {
    get { self[ChillPageSurfaceCoordinatorKey.self] }
    set { self[ChillPageSurfaceCoordinatorKey.self] = newValue }
  }
}

func newAnnotationScopeID() -> AnnotationScopeID {
  try! AnnotationScopeID(UUIDv7.generate().uuidString.lowercased())
}

func newPageInstanceID() -> PageInstanceID {
  try! PageInstanceID(UUIDv7.generate().uuidString.lowercased())
}

func newElementInstanceID() -> ElementInstanceID {
  try! ElementInstanceID(UUIDv7.generate().uuidString.lowercased())
}

func newSurfaceID() -> SurfaceID {
  try! SurfaceID(UUIDv7.generate().uuidString.lowercased())
}
