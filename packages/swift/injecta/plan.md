# Injecta (Swift)

Compile-time dependency injection for Swift 6: macros write the wiring as initializer calls, a
build-tool plugin checks the whole graph and generates the `Needs` conformances; the Swift
counterpart of the Rust crate `injecta`.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P1 | 3 | 0% | |
| T5 | todo | P2 | 3 | 0% | |
| T7 | todo | P3 | 2 | 0% | |
| T8 | todo | P1 | 2 | 0% | |
| T9 | todo | P3 | 3 | 0% | |

### T2. Try Injecta on Cox's app-wide services

Wire `coreClient`, `secretStore`, `inboxClient` and the stores built from them in a branch of the
Cox desktop app (`desktop/macos`) with a `@MainActor` app container and a per-session scope, next
to PR #184's swift-dependencies version, and compare lines, build time and test ergonomics. Done
when both versions build and their test suites pass.

### T5. Async inputs helper

A documented pattern and helper for building async/throwing values before the container
(`try await AppGraph.make(...)`). Done when GUIDE.md shows it and a test covers it.

### T7. Linux support

`Synchronization` is available on Linux; run the tests there and drop Darwin-only assumptions in the
benchmarks. Done when `swift test` passes in the swift:6.4 container.

### T8. swift-syntax prebuilt in clean builds

A clean release build of a target using Injecta compiled swift-syntax from source (234 objects,
about 194 of 202 s) with SwiftPM 6.4's default `swiftbuild` system. Find out whether
`--build-system native`, Xcode builds, or a different target layout get the swift.org prebuilt,
and document the result. Done when README states the measured clean build time per build system.

### T9. Needle in the benchmarks

Build Needle's generator in a separate scratch package (it pins swift-syntax 510.x, which cannot
share a graph with 604), commit its generated code for the benchmark graph, and add Needle to
`di-bench` and `measure.sh`. Done when the README table has a Needle column.

