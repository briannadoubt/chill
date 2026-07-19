// swift-tools-version: 6.4

import CompilerPluginSupport
import PackageDescription

let package = Package(
  name: "Chill",
  platforms: [
    .iOS(.v17),
    .macOS(.v14),
    .tvOS(.v17),
    .watchOS(.v10),
    .visionOS(.v1),
  ],
  products: [
    .library(name: "ChillCore", targets: ["ChillCore"]),
    .library(name: "ChillSwiftUI", targets: ["ChillSwiftUI"]),
    .library(name: "ChillUIKit", targets: ["ChillUIKit"]),
    .library(name: "ChillNetworking", targets: ["ChillNetworking"]),
    .library(name: "ChillExport", targets: ["ChillExport"]),
    .library(name: "ChillReplay", targets: ["ChillReplay"]),
    .library(name: "Chill", targets: ["Chill"]),
  ],
  dependencies: [
    .package(
      url: "https://github.com/swiftlang/swift-syntax.git",
      exact: "604.0.0-prerelease-2026-06-05"
    )
  ],
  targets: [
    .target(name: "CChillAtomics"),
    .target(
      name: "CChillCompression",
      linkerSettings: [.linkedLibrary("z")]
    ),
    .target(name: "ChillCore", dependencies: ["CChillAtomics"]),
    .target(name: "ChillSwiftUI", dependencies: ["ChillCore"]),
    .target(name: "ChillUIKit", dependencies: ["ChillCore"]),
    .target(name: "ChillNetworking", dependencies: ["ChillCore"]),
    .target(
      name: "ChillExport",
      dependencies: ["ChillCore", "CChillCompression"]
    ),
    .target(
      name: "ChillReplay",
      dependencies: ["ChillCore", "ChillUIKit", "CChillCompression"],
      linkerSettings: [.linkedFramework("Security")]
    ),
    .macro(
      name: "ChillMacrosPlugin",
      dependencies: [
        .product(name: "SwiftCompilerPlugin", package: "swift-syntax"),
        .product(name: "SwiftDiagnostics", package: "swift-syntax"),
        .product(name: "SwiftSyntax", package: "swift-syntax"),
        .product(name: "SwiftSyntaxBuilder", package: "swift-syntax"),
        .product(name: "SwiftSyntaxMacros", package: "swift-syntax"),
      ]
    ),
    .target(
      name: "Chill",
      dependencies: [
        "ChillCore", "ChillSwiftUI", "ChillUIKit", "ChillNetworking",
        "ChillExport", "ChillMacrosPlugin",
      ]
    ),
    .executableTarget(
      name: "ChillCoreBenchmarks",
      dependencies: ["ChillCore"]
    ),
    .executableTarget(
      name: "ChillReplayBenchmarks",
      dependencies: ["ChillCore", "ChillReplay"]
    ),
    .executableTarget(
      name: "ChillConformanceRunner",
      dependencies: [
        "ChillCore", "ChillExport", "ChillNetworking", "ChillReplay",
      ]
    ),
    .executableTarget(
      name: "ChillBackendE2E",
      dependencies: ["Chill"]
    ),
    .testTarget(
      name: "ChillCoreTests",
      dependencies: ["ChillCore"]
    ),
    .testTarget(
      name: "ChillMacroTests",
      dependencies: [
        "Chill",
        "ChillCore",
        "ChillMacrosPlugin",
        .product(name: "SwiftSyntaxMacrosTestSupport", package: "swift-syntax"),
      ]
    ),
    .testTarget(
      name: "ChillSwiftUITests",
      dependencies: ["ChillCore", "ChillSwiftUI"]
    ),
    .testTarget(
      name: "ChillUIKitTests",
      dependencies: ["ChillCore", "ChillUIKit"]
    ),
    .testTarget(
      name: "ChillNetworkingTests",
      dependencies: ["ChillCore", "ChillNetworking"]
    ),
    .testTarget(
      name: "ChillExportTests",
      dependencies: ["ChillCore", "ChillExport"]
    ),
    .testTarget(
      name: "ChillReplayTests",
      dependencies: [
        "ChillCore", "ChillReplay", "ChillSwiftUI", "ChillUIKit",
      ]
    ),
    .testTarget(
      name: "ChillValidationTests",
      dependencies: ["Chill", "ChillCore"],
      resources: [.process("Fixtures")]
    ),
  ]
)
