// swift-tools-version: 6.2
// Comparison benchmarks, a separate package so the library never depends on the libraries it is
// measured against. `di-bench` times resolution; the `size-*` executables wire the same graph with
// one library each, for release binary size and clean build time (`measure.sh`).
import PackageDescription

let package = Package(
  name: "InjectaBenchmarks",
  platforms: [.macOS(.v15)],
  dependencies: [
    .package(path: ".."),
    // The version Cox's PR #184 pins.
    .package(url: "https://github.com/pointfreeco/swift-dependencies", exact: "1.17.1"),
    .package(url: "https://github.com/Swinject/Swinject", exact: "2.10.0"),
    .package(url: "https://github.com/hmlongco/Factory", exact: "3.4.1"),
    // Deprecated by its author in favour of Factory; measured because apps still ship it.
    .package(url: "https://github.com/hmlongco/Resolver", exact: "1.5.1"),
  ],
  targets: [
    .executableTarget(
      name: "di-bench",
      dependencies: [
        .product(name: "Injecta", package: "injecta-swift"),
        .product(name: "Dependencies", package: "swift-dependencies"),
        .product(name: "Swinject", package: "Swinject"),
        .product(name: "FactoryKit", package: "Factory"),
        .product(name: "Resolver", package: "Resolver"),
      ],
      // Top-level benchmark state is shared across threads on purpose; the library is Swift 6.
      swiftSettings: [.swiftLanguageMode(.v5)],
      plugins: [.plugin(name: "InjectaCheckPlugin", package: "injecta-swift")]
    ),
    .target(name: "BenchModel"),
    .executableTarget(name: "size-manual", dependencies: ["BenchModel"]),
    .executableTarget(
      name: "size-injecta",
      dependencies: ["BenchModel", .product(name: "Injecta", package: "injecta-swift")]),
    .executableTarget(
      name: "size-factory",
      dependencies: ["BenchModel", .product(name: "FactoryKit", package: "Factory")]),
    .executableTarget(
      name: "size-swinject",
      dependencies: ["BenchModel", .product(name: "Swinject", package: "Swinject")]),
    .executableTarget(
      name: "size-resolver",
      dependencies: ["BenchModel", .product(name: "Resolver", package: "Resolver")],
      // Resolver predates Sendable; its documented style captures the resolver in factories.
      swiftSettings: [.swiftLanguageMode(.v5)]),
    .executableTarget(
      name: "size-dependencies",
      dependencies: ["BenchModel", .product(name: "Dependencies", package: "swift-dependencies")]),
  ]
)
