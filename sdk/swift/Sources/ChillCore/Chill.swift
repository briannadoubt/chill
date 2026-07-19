import CChillAtomics
import os

public enum Chill {
  private static let storage = OSAllocatedUnfairLock<ChillRuntime?>(
    initialState: nil
  )

  public static var isEnabled: Bool {
    chill_runtime_is_enabled()
  }

  public static func configure(_ runtime: ChillRuntime) {
    storage.withLock { $0 = runtime }
    chill_runtime_set_enabled(runtime.configuration.enabled)
  }

  public static func disable() {
    chill_runtime_set_enabled(false)
    storage.withLock { $0 = nil }
  }

  package static func currentRuntime() -> ChillRuntime? {
    guard chill_runtime_is_enabled() else { return nil }
    return storage.withLock { $0 }
  }
}
