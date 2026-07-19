// swift-tools-version: 6.4

import PackageDescription

let package = Package(
  name: "ChillAppleExamples",
  platforms: [
    .iOS(.v17),
    .macOS(.v14),
  ],
  products: [
    .executable(
      name: "ChillSwiftUISample",
      targets: ["ChillSwiftUISample"]
    ),
    .executable(
      name: "ChillUIKitSample",
      targets: ["ChillUIKitSample"]
    ),
  ],
  dependencies: [
    .package(name: "Chill", path: "..")
  ],
  targets: [
    .target(
      name: "ChillSampleSupport",
      dependencies: [
        .product(name: "Chill", package: "Chill"),
        .product(name: "ChillReplay", package: "Chill"),
      ]
    ),
    .executableTarget(
      name: "ChillSwiftUISample",
      dependencies: [
        "ChillSampleSupport",
        .product(name: "Chill", package: "Chill"),
      ]
    ),
    .executableTarget(
      name: "ChillUIKitSample",
      dependencies: [
        "ChillSampleSupport",
        .product(name: "Chill", package: "Chill"),
      ]
    ),
  ]
)
