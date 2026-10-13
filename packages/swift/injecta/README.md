# Injecta (Swift)

Compile-time dependency injection for Swift 6. You declare a container with `@Container` and
mark the types it builds `@Injectable`. The macros then write the wiring as ordinary initializer
calls, so reading an entry is a property read. A build-tool plugin checks the whole graph across
files. It reports a missing entry, a dependency cycle or a scope violation as a compile error at
the line that causes it. This is the Swift counterpart of the Rust crate `injecta`.

Status: experimental, local package (no remote yet). Requires Swift 6.2+ (developed with
Xcode 27 / Swift 6.4) on macOS 15, iOS 18, tvOS 18, watchOS 11 or visionOS 2.

```swift
import Injecta

@Injectable
final class Database: Sendable {
  init(config: Config, logger: any Logger) { ... }
}

@Injectable
struct UserRepo: Sendable {
  let db: Database
  let logger: any Logger
}

@Container
final class AppGraph: Sendable {
  let config: Config                                         // init input
  @Provides(.singleton) func makeLogger() -> any Logger { ConsoleLogger() }
  @Singleton var db: Database                                // one per container, lazy
  @Transient var repo: UserRepo                              // new on every read
}

let graph = AppGraph(config: Config())
let repo = graph.repo                                        // UserRepo(db: graph.db, logger: graph.logger)

// Tests and previews: replace any entry, no global state.
let test = AppGraph(config: .test, overrides: .init(logger: SilentLogger()))
```

## Why

- **No hot-path machinery.** A singleton read is one acquiring atomic load. A transient read is
  the initializer call plus a branch for overrides. There is no type-keyed registry, dictionary,
  lock (after the first build) or task-local lookup.
- **Errors at compile time.** Missing entries, cycles, misuse of attributes and captive
  dependencies (a singleton holding a transient, reported as a warning) are diagnosed by the
  macros within a container and by the plugin across files.
- **Little boilerplate.** You write one attribute per type and one line per entry. You don't
  write a registration, a key type or a property wrapper at the use site.
- **Swift 6 native.** Works under strict concurrency, with `Sendable` containers and `@MainActor`
  containers for UI models (isolated conformances, SE-0470).
- **Scopes as types.** A child container (session, request, window) takes its parent as an input
  and forwards entries with `@Forward`.

Docs:

- [GUIDE.md](GUIDE.md): the full guide.
- [AGENTS.md](AGENTS.md): rules for coding agents. `CLAUDE.md` is a symlink to it.
- [llms.txt](llms.txt) and [llms-full.txt](llms-full.txt): generated from GUIDE.md and AGENTS.md
  by `INJECTA_BLESS=1 swift test --filter DocsTests`. A test fails if they go stale.

## Package layout

| Target | Kind | Role |
| --- | --- | --- |
| `Injecta` | library | Runtime (`Once`, `Container`, `Injectable`, `Provision`) and the macro declarations |
| `InjectaGraph` | library | Graph model and checks (missing, cycle, captive), DOT export; no dependencies |
| `InjectaSyntax` | library (internal use) | Reads declarations into the model; used by `injecta-check` |
| `InjectaMacros` | macro | `@Injectable`, `@Inject`, `@Container`, `@Singleton`, `@Transient`, `@Provides`, `@Forward`; compiles the same reader via the `Reader` symlink |
| `injecta-check` | executable | Whole-target checks and generated `Needs` conformances |
| `InjectaCheckPlugin` | build-tool plugin | Runs `injecta-check` in SwiftPM and Xcode builds |
| `Benchmarks/` | separate package | Comparison harness; never a dependency of the library |

## Comparison with other frameworks

### What is compared

| Framework | Version | How it resolves | In the benchmark |
| --- | --- | --- | --- |
| hand-written (`manual`) | — | Init calls written by hand; singletons built in `init` | yes (baseline) |
| Injecta | this package | Macro-generated init calls; `Once` per singleton | yes |
| [swift-dependencies](https://github.com/pointfreeco/swift-dependencies) | 1.17.1 | `@Dependency` reads `DependencyValues` from a task-local; keys cached per type | yes |
| [Factory](https://github.com/hmlongco/Factory) | 3.4.1 (`FactoryKit`) | Computed `Factory<T>` properties on a container; scope caches keyed by factory | yes |
| [Swinject](https://github.com/Swinject/Swinject) | 2.10.0 | Runtime registry keyed by type (and name); `synchronize()` for thread safety | yes |
| [Resolver](https://github.com/hmlongco/Resolver) | 1.5.1 | Runtime registry keyed by type, locked; deprecated by its author in favour of Factory | yes |
| [Needle](https://github.com/uber/needle) | v0.25.1 | Generated code from an external generator, components per scope | **no**, see below |

**Why Needle is not measured.** Needle has two parts. `NeedleFoundation` is a SwiftPM library.
The code generator is a separate command-line tool, installed with `brew install needle` or from
Carthage checkouts. Needle's root `Package.swift` has no SwiftPM build plugin, so a SwiftPM
package can't run the generator by itself. The generator's own package (`Generator/Package.swift`)
pins `swift-syntax` from 510.0.0 up to the next major version. That can't resolve in one SwiftPM
graph with the swift-syntax 604 that Injecta needs. Measuring Needle therefore means one of two
things:

- installing the generator globally, which this package's rules forbid;
- building the generator in a separate package and committing its generated output.

Neither is done yet (see `todo.md`). Sources:
[README](https://github.com/uber/needle/blob/master/README.md),
[Package.swift](https://github.com/uber/needle/blob/master/Package.swift) and
[Generator/Package.swift](https://github.com/uber/needle/blob/master/Generator/Package.swift),
read 2026-10-08.

Other libraries that were compared on design but not benchmarked:

- [SafeDI](https://github.com/dfed/SafeDI): compile-time graph via macros and a plugin; the
  closest design.
- [Cleanse](https://github.com/square/Cleanse): archived.
- [Weaver](https://github.com/scribd/Weaver) and [Knit](https://github.com/cashapp/knit):
  generators.

### How the benchmarks work

- **Graph.** Every library wires the same graph from `Benchmarks/Sources/di-bench/Model.swift`:
  - singletons `Config`, `Logger` (`any Logger`), `Database` and `Cache`;
  - transients `UserRepo`, `UserService` and `Handler`;
  - a chain of eleven structs `L0`…`L10`.

  Each library is used in the style its documentation shows (`Benchmarks/Sources/di-bench/Wiring.swift`,
  `ResolverWiring.swift`). swift-dependencies has no transient providers, so its transients are
  init calls that read `@Dependency` values, the way an app writes them.
- **Timing (`di-bench`).**
  - Release build. Each case runs a loop of N operations once to warm up, then 15 timed runs with
    `ContinuousClock`, and reports the median nanoseconds per operation.
  - Every operation feeds a checksum into a non-inlined sink, so the optimizer can't drop the work.
  - The concurrent case runs 8 threads with `DispatchQueue.concurrentPerform` (9 runs, median),
    each with its own cache-line-padded sink.
  - Swinject uses `synchronize()` there, its documented thread-safe resolver.
  - `n/a` means the library has no equivalent operation in its documented API.
- **Cases.**
  - *singleton (cached)*: read an already-built singleton.
  - *graph*: build a `Handler` (3 transients over 3 singletons).
  - *chain*: build `L10` through 10 transient levels.
  - *cold start*: create a new container and build the first `Handler`.
  - *override scope*: create a scope that replaces the logger, then build a `Handler`. For
    Injecta that's `Overrides` plus a new container; for swift-dependencies it's `withDependencies`.
  - *8 threads*: the graph case from 8 threads at once, per operation.
- **Size and build time.**
  - For each library, `measure.sh` builds an executable (`Benchmarks/Sources/size-*`) that wires
    the singletons and transients and prints one checksum.
  - Each build is a clean release build in its own scratch directory, after `swift package
    resolve`, so the time is compilation, not download.
  - The binary is copied and `strip`ped before measuring.
  - For Injecta it then touches only Injecta's own sources and rebuilds. That separates
    Injecta's compile time from swift-syntax's, and it counts the swift-syntax object files.
- **Allocations.** Not measured.
- **Reproduce.**

  ```sh
  cd Benchmarks && ./measure.sh   # about 8 minutes; writes results/<date>.md
  ```

### Results

Run 2026-10-08 16:33 EEST on an Apple M3 Max (16 cores) with Xcode 27.0 / Swift 6.4
(swiftlang-6.4.0.34.1). The machine was busy: load average 14.5 before and 26.0 after on 16
cores. Absolute numbers will vary between runs. An earlier run the same day, before Resolver
was added, gave Injecta numbers within 16% of these. Raw output: [Benchmarks/results/2026-10-08.md](Benchmarks/results/2026-10-08.md).

**Resolve time** (ns per operation, median of 15 runs, release):

| case | manual | Injecta | swift-dependencies | Factory | Swinject | Resolver |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| singleton (cached) | 0.8 | 4.1 | 565 | 629 | 699 | 150 |
| graph (3 transients + 3 singletons) | 13.0 | 29.2 | 3501 | 4077 | 4955 | 1631 |
| chain of 10 transients | 2.0 | 4.3 | n/a | 2732 | 8372 | 1727 |
| cold start (new container + first graph) | 35.1 | 404 | n/a | 4796 | 16217 | 7289 |
| override scope + graph | n/a | 442 | 3246 | n/a | n/a | n/a |
| graph, 8 threads at once (per op) | 338 | 547 | 10685 | 11286 | 7120 | 3025 |

Notes on what these numbers mean:

- The *chain* row for manual and Injecta is far below the cost of building eleven values. Both
  chains are plain struct initializers, and the optimizer flattens them. The row shows that
  Injecta's generated wiring is as transparent to the optimizer as hand-written code.
- *Cold start* for Injecta includes allocating the container class and one `Once` (atomic plus
  mutex) per singleton. Hand-written code builds its singletons as plain stored properties.
- In the *8 threads* row, manual and Injecta are dominated by reference-count traffic on the
  shared singletons, which every library pays as well.

**Clean release build time and stripped binary size** of the `size-*` executables:

| library | stripped size (bytes) | delta vs manual (bytes) | clean build (s) |
| --- | ---: | ---: | ---: |
| manual | 52,520 | 0 | 8 |
| Injecta | 95,376 | 42,856 | 202 |
| — Injecta's own modules only (rebuild after touching its sources) | | | 8 |
| Factory | 195,536 | 143,016 | 13 |
| Swinject | 190,256 | 137,736 | 11 |
| Resolver | 141,456 | 88,936 | 11 |
| swift-dependencies | 702,144 | 649,624 | 16 |

**Injecta's build time is dominated by swift-syntax. This was verified, not assumed.**

- The clean Injecta build compiled 234 swift-syntax object files from source: 20 swift-syntax
  targets, including `SwiftSyntax`, `SwiftParser`, `SwiftSyntaxMacros` and `SwiftCompilerPlugin`.
- Rebuilding only Injecta's own modules, after touching their sources, took 8 s. That's the
  same as the hand-written baseline's whole clean build.
- So about 194 of the 202 s are swift-syntax.
- It's a one-time cost per clean build or CI cache miss. Incremental builds don't recompile
  swift-syntax.
- SwiftPM documents prebuilt swift-syntax for macros (`--enable-experimental-prebuilts`, the
  default). Even so, SwiftPM 6.4's default build system (`swiftbuild`) compiled it from source
  here, even after the macro target was changed to depend on swift-syntax products directly.
  Whether `--build-system native` uses the prebuilt was not checked.
- The swift-dependencies row uses only the `Dependencies` product. Its `@DependencyClient`
  macros (`DependenciesMacros`) also depend on swift-syntax and were not measured.

### Summary

- **Resolve time.** Compared with hand-written code, Injecta took about 2.2× as long for the
  graph and chain cases, 5× for a cached singleton (4.1 ns vs 0.8 ns), 11.5× for a cold start
  and 1.6× under 8 threads.
- **Versus the fastest runtime library per case.** Injecta was 37× faster for a cached
  singleton (Resolver), 56× for the graph (Resolver), 12× for a cold start (Factory) and 5.5×
  under 8 threads (Resolver).
- **Versus swift-dependencies.** Injecta was 138× faster for a cached singleton, 120× for the
  graph and 7× for an override scope.
- **Binary size.** Injecta's binary was the smallest of the libraries.
- **Build cost.** Injecta pays for that with swift-syntax compile time on clean builds. The
  runtime containers have no such cost; swift-dependencies has it too as soon as an app uses
  its macros.
- **Scale.** For an app with a handful of services that resolves at launch, every library here
  is fast enough. The difference matters for resolves on hot paths and for per-request or
  per-session scopes created often.

## Development

```sh
swift test                                        # all tests, including macro expansion tests
INJECTA_BLESS=1 swift test --filter DocsTests     # regenerate llms.txt and llms-full.txt
cd Benchmarks && ./measure.sh                     # benchmarks
```

GitHub Actions (`.github/workflows/ci.yml`) runs `swift build` and `swift test` on macOS
(Xcode 27) and Linux (Swift 6.4). The test run includes the docs-freshness check.

Project files: `plan.md` (tasks), `todo.md`, `done.md`, `roadmap.md`, `ideas.md`,
`toolchain.md`.
