# Tasks

Planned work items (details in `plan.md`):

- T1. Reusable modules
- T2. Runtime guard against factory cycles
- T3. Nested scopes
- T4. Async-ready hook

## Prioritized tasks

Priorities: **P0** blocks a first release or fixes a hazard, **P1** is the next feature set, **P2** is worth doing when there is a user asking. Effort: S (hours), M (a day or two), L (a week).

### P0

| # | Task | Effort | Done when |
| --- | --- | --- | --- |
| P0-1 | **crates.io prerequisites.** README doctest path is fixed (the crate reads its own `README.md`, and `tests/docs.rs` skips when `AGENTS.md` is absent). Still open: `documentation`/`homepage` metadata and docs.rs `all-features`; publish order (`injecta-macros` first, then `injecta` with `version = "=x.y.z"`); a release-plz/CHANGELOG decision for the two crates; license (the repo default is GPL-3.0-or-later, several sibling crates are MIT). | M | `cargo publish --dry-run -p injecta-macros` and, after a macros release, `-p injecta` pass in CI |
| P0-2 | **CI wiring.** Both crates are workspace members, so CI's fmt, clippy and `cargo test --workspace` already cover them on Linux, macOS and Windows; `injecta/bench` is excluded. Add the MSRV check (`cargo +1.86 check -p injecta`) and the `no_std` clippy (`--no-default-features --features macros`) somewhere CI runs them, and a publish dry-run once P0-1 lands. | S | a red MSRV or `no_std` build fails a PR |
| P0-3 | **Runtime guard against factory cycles** (was T2). A factory that resolves its own entry recurses until the stack overflows (`SingleThread`) or deadlocks (`ThreadSafe`). Track "initializing" per cell on the cold path only and panic with the provider name. | M | a test shows the message for both storages; the warm resolve benchmarks do not move |
| P0-4 | **Diagnostics.** (a) The cycle error is rustc's `cycle detected when simplifying constant ... DEPTH` dump: add a `container!`-level pass that finds direct cycles among non-factory entries and reports `A -> B -> A` with spans before rustc does. (b) `.with(value)` on a transient prints the type as `_`: give `with` a typed fallback path (or a `with::<T>` turbofish in the hint) so the message names the type. | M | `tests/ui/dependency_cycle.stderr` and `override_transient.stderr` name the types |

### P1

| # | Task | Effort | Done when |
| --- | --- | --- | --- |
| P1-1 | **Modules / submodules** (was T1): `injecta::module!` exported by a library crate and included by containers; duplicates across modules are compile errors. | L | two containers include one module; a compile-fail case covers a duplicate from a module |
| P1-2 | **Factories as first-class citizens**: factories that declare their dependencies (`singleton Arc<Client> = \|c: deps(Config)\| ..`) so they take part in the compile-time cycle and depth check (today they count as depth 0). | M | a factory cycle is a compile error |
| P1-3 | **Async providers / ready step** (was T4): `App::ready().await` runs async initializers of marked singletons up front; `resolve` stays sync. | M | an example initializes a pool with tokio before serving |
| P1-4 | **Lifecycle start/stop** (Go fx style): optional `start`/`stop` hooks run in dependency order (`stop` in reverse) for servers, consumers and pools. | M | a test records start/stop order for a 3-level graph |
| P1-5 | **Multi-bindings**: `Vec<Arc<dyn Plugin>>` collected from several entries. | M | plugins from two entries arrive in declaration order |
| P1-6 | **Axum / Tokio integration** (separate `injecta-axum` crate): `State<Arc<App>>` + an extractor that opens a request scope; needs an owned scope variant (`Request<Arc<App>>`) because today's scope borrows `&App` and axum handlers need `'static`. `SingleThread` documented for `current_thread` runtimes. | L | an axum example serves a request through a scoped handler |
| P1-7 | **Nested scopes** (was T3): `scope Session in Request { .. }`. | M | a three-level example resolves from all levels; captive check per level |
| P1-8 | **Docs with more examples**: testing with overrides, generic library code over `Provide`/`ProvideRef`, `no_std`, axum, a migration guide from shaku and from hand-written wiring. | M | each example is a doctest or an `examples/` binary run in CI |

### P2

| # | Task | Effort | Done when |
| --- | --- | --- | --- |
| P2-1 | `Lazy<T>` dependency wrapper for expensive optional services. | S | a test shows the inner value built on first use only |
| P2-2 | Graph export (`App::graph()` as DOT/Mermaid) from `PROVIDERS` plus per-entry dependencies. | M | the example prints a graph that renders |
| P2-3 | Speedup ideas from `docs/performance.md` (P1/P2 items there). | — | each lands only with before/after numbers |
| P2-4 | `cargo xtask` that turns `container!` compile errors into one-line agent hints. | S | hint output covered by a test |
