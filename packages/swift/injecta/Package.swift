// swift-tools-version: 6.2
// Injecta: compile-time dependency injection for Swift (the Swift counterpart of the Rust crate
// `injecta`). Macros wire the graph at compile time; the build-tool plugin checks the whole graph
// across files; the runtime is a few hundred lines with no registry on the resolve path.
import CompilerPluginSupport
import PackageDescription

let package = Package(
  name: "Injecta",
  // `AtomicLazyReference` and `Mutex` (Synchronization) set the floor.
  platforms: [.macOS(.v15), .iOS(.v18), .tvOS(.v18), .watchOS(.v11), .visionOS(.v2)],
  products: [
    .library(name: "Injecta", targets: ["Injecta"]),
    .library(name: "InjectaGraph", targets: ["InjectaGraph"]),
    .executable(name: "injecta-check", targets: ["injecta-check"]),
    .plugin(name: "InjectaCheckPlugin", targets: ["InjectaCheckPlugin"]),
  ],
  dependencies: [
    // Matches the Swift 6.4 toolchain, so SwiftPM/Xcode can use the swift.org prebuilt.
    .package(url: "https://github.com/swiftlang/swift-syntax.git", from: "604.0.0")
  ],
  targets: [
    // The graph model and its checks (missing, cycle, captive). No dependencies, shared by the
    // runtime, the macros and the checker so the rules exist once.
    .target(name: "InjectaGraph"),
    // What a declaration means to Injecta, read from SwiftSyntax. Shared by the macros (one
    // declaration at a time) and `injecta-check` (every file of a target).
    .target(
      name: "InjectaSyntax",
      dependencies: [
        "InjectaGraph",
        .product(name: "SwiftSyntax", package: "swift-syntax"),
      ]
    ),
    // `Reader` is a symlink to `Sources/InjectaSyntax`: the macro compiles the same reader as
    // `injecta-check` without depending on a library target that imports swift-syntax, which
    // would make SwiftPM build swift-syntax from source instead of using the swift.org prebuilt
    // (measured: 186 s versus 15 s for a clean release build).
    .macro(
      name: "InjectaMacros",
      dependencies: [
        "InjectaGraph",
        .product(name: "SwiftSyntax", package: "swift-syntax"),
        .product(name: "SwiftSyntaxBuilder", package: "swift-syntax"),
        .product(name: "SwiftSyntaxMacros", package: "swift-syntax"),
        .product(name: "SwiftCompilerPlugin", package: "swift-syntax"),
        .product(name: "SwiftDiagnostics", package: "swift-syntax"),
      ]
    ),
    .target(name: "Injecta", dependencies: ["InjectaGraph", "InjectaMacros"]),
    .executableTarget(
      name: "injecta-check",
      dependencies: [
        "InjectaSyntax",
        "InjectaGraph",
        .product(name: "SwiftParser", package: "swift-syntax"),
        .product(name: "SwiftSyntax", package: "swift-syntax"),
      ]
    ),
    .plugin(
      name: "InjectaCheckPlugin",
      capability: .buildTool(),
      dependencies: ["injecta-check"]
    ),
    .testTarget(name: "InjectaGraphTests", dependencies: ["InjectaGraph"]),
    .testTarget(
      name: "InjectaSyntaxTests",
      dependencies: [
        "InjectaSyntax",
        .product(name: "SwiftParser", package: "swift-syntax"),
      ]
    ),
    .testTarget(
      name: "InjectaMacrosTests",
      dependencies: [
        "InjectaMacros",
        .product(name: "SwiftParser", package: "swift-syntax"),
        .product(name: "SwiftSyntaxMacroExpansion", package: "swift-syntax"),
        .product(name: "SwiftSyntaxMacrosTestSupport", package: "swift-syntax"),
      ]
    ),
    .testTarget(name: "InjectaTests", dependencies: ["Injecta"], plugins: ["InjectaCheckPlugin"]),
    // No plugin: a debug build must trap with the setup step instead of a bare conformance error.
    .testTarget(name: "InjectaNoPluginTests", dependencies: ["Injecta"]),
  ]
)
