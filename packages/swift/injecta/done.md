# Done

### T1. Skeleton: macros, runtime, checker plugin, tests, benchmarks, docs

`@Injectable`, `@Inject`, `@Container`, `@Singleton`, `@Transient`, `@Provides`, `@Forward`; the
inline lock-free `Once` cell; `InjectaGraph` checks; `injecta-check` and `InjectaCheckPlugin`
(whole-target checks and generated `Needs` conformances); tests for the graph, the syntax reader,
macro expansions and diagnostics, runtime behaviour and docs freshness; the comparison benchmarks
against hand-written wiring, swift-dependencies, Factory and Swinject; GUIDE.md, AGENTS.md,
llms.txt.

### T3. Lazy and provider entries

`Lazy<T>` and `() -> T` (including `@Sendable () -> T`) dependencies. The macro passes
`Lazy { needs.entry }` or `{ needs.entry }`, and the graph lists them as deferred (`~entry`):
they are not cycle edges and they do not capture a transient. `() -> T` builds a new value per
call; `Lazy<T>` builds one, on the first `.value`.

### T4. Better messages for conformance errors without the plugin

A debug build with no `Needs` conformance (the plugin was not attached, and the line was not
written by hand) compiles and traps on `init` with `missingPluginMessage`, which names
`InjectaCheckPlugin` and the conformance line. The check is the auto-wired entries of
`injectaGraph` against the container's conformances. Release builds keep the static
`Type(injecting: self)` call. Covered by `InjectaNoPluginTests`.

### T6. CI

GitHub Actions (`.github/workflows/ci.yml`) runs `swift build` and `swift test` on macOS with
Xcode 27 (`runs-on: xcode-27`; `macos-latest` is the macOS 26 image and does not ship Xcode 27)
and on Linux in the `swift:6.4.0-noble` container. `swift test` includes the docs-freshness
check (`DocsTests`).
