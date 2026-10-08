# injecta

Compile-time dependency injection for Rust. Declare a container once, derive `Injectable` on your types, call `resolve`. A missing provider, a dependency cycle or a singleton that captures a request-scoped value is a compile error with a fix hint, and resolving costs about the same as wiring by hand.

```rust
use std::sync::Arc;
use injecta::{Injectable, Resolve};

#[derive(Clone)]
struct Config { url: String }

#[derive(Injectable)]
struct Database { config: Config }

#[derive(Injectable)]
struct UserRepo { db: Arc<Database> }

#[derive(Injectable)]
struct Handler { repo: UserRepo }

injecta::container! {
    pub struct App {
        instance Config,          // passed to App::new
        singleton Arc<Database>,  // created once, on first use
        transient UserRepo,       // created on every resolve
    }
    pub scope Request {           // per-request child container
        transient Handler,
    }
}

let app = App::new(Config { url: "postgres://localhost".into() });
let request = Request::new(&app);
let handler = request.resolve::<Handler>();
assert_eq!(handler.repo.db.config.url, "postgres://localhost");
```

Status: 0.1.0, local only (not published). The guide with every feature and runnable examples is [`GUIDE.md`](GUIDE.md); it is also the crate's rustdoc.

## Why another DI crate

The existing crates make you pick two of: compile-time checking, low ceremony, scopes. shaku checks at compile time but needs a trait per service and `Arc<dyn>`/`Box<dyn>` everywhere; nject is zero-cost but has no lazy singletons or scopes and wires singletons by hand; teloc is compile-time but unmaintained since 2021 and its errors are frunk HList dumps; dill, syrette, ferrunix and springtime are runtime registries that fail at startup or at resolve. injecta keeps the compile-time guarantees and the zero-cost resolve of nject/teloc, adds lazy singletons, scopes and test overrides, and turns misuse into short compiler messages that say what to write.

## Benchmarks

Same 7-type graph in every implementation (1 instance, 3 singletons including an `Arc<dyn Logger>`, 3 transients), sources in [`bench/`](bench). Local measurements on Ivan's MacBook Pro (Apple M3 Max, 16 cores, 64 GB, macOS 27.0.1), rustc 1.99.0, `--release` (opt-level 3, no LTO), divan 0.1.21 with 500 samples, 2026-10-08. The machine was heavily loaded (load average 280–375) during the build-time runs, so treat build times as rough.

| Implementation | `Handler` (3 transients) | warm singleton | build container + first resolve |
| --- | ---: | ---: | ---: |
| hand-written | 8.5 ns | 3.6–4.0 ns | 71–78 ns |
| **injecta** | 9.1–9.5 ns | 3.8–4.0 ns | 90–92 ns |
| nject 0.5.1 | 8.2–8.6 ns | 3.8–4.0 ns | 75–78 ns |
| teloc 0.2.0 | 10.3 ns | 3.8–4.0 ns | 84–87 ns |
| shaku 0.6.3 | 68–69 ns | 3.9–4.1 ns | 625 ns |
| dill 0.17.1 | 494–515 ns | 46 ns | 2.2–2.7 µs |

Medians of two runs. injecta's remaining gap to hand-written wiring is the lazy `OnceLock` check per singleton (the hand-written version builds singletons eagerly).

| Implementation | release binary | stripped | lines (total / wiring) | crate-only clean build, CPU s | cold build incl. deps, CPU s |
| --- | ---: | ---: | ---: | ---: | ---: |
| hand-written | 436,688 B | 343,144 B | 104 / 46 | 0.37 | 0.55 |
| **injecta** | 440,096 B | 343,096 B | 97 / 23 | 0.46 | 4.15 |
| nject | 436,608 B | 343,120 B | 99 / 37 | 0.35 | 3.81 |
| teloc | 461,088 B | 360,048 B | 106 / 17 | 0.43 | 9.35 |
| shaku | 490,144 B | 378,144 B | 165 / 22 | 0.80 | 5.39 |
| dill | 568,560 B | 431,056 B | 97 / 21 | 1.36 | 10.25 |

Raw output: `bench/results/`. Reproduce with `just bench` (resolve) and `bench/measure.sh` (build time, size, lines).

## Performance

A resolve compiles to the constructor calls plus, per shared entry, one load and branch on its cell and the `Arc` clone the hand-written version also does. Full criterion results (cold start, lazy vs cached vs borrowed singletons, transients, deep chains, loops, 1–8 threads, scopes, overrides, the cross-crate comparison), the optimizations and the remaining ideas: [`docs/performance.md`](docs/performance.md).

Optimization in this round, measured on the same machine (M3 Max, rustc 1.99.0, criterion 0.8.2, shared machine with load average 58–468):

| Path | `resolve` (clone) | `get` (borrow, new) | Speedup |
| --- | ---: | ---: | ---: |
| warm `Arc` singleton, one access | 5.65 ns | 0.756 ns | 7.5x |
| 1 000 accesses in a loop | 5.27 µs | 0.628 µs | 8.4x |
| one singleton read from 8 threads | 21.2 M/s | 1 048 M/s | 49x |
| scoped value in a request scope | 5.21 ns | 0.733 ns | 7.1x |

`get::<T>()` lends an `instance`, `singleton` or `scoped` value without touching its reference count; `resolve::<T>()` still returns an owned value. The refactor behind it leaves the generated machine code for `resolve` identical (checked on aarch64) and the binary size unchanged. `SingleThread` storage is about 20% faster than the default on first initialization (37.7 vs 48.6 ns).

## Documentation for humans and agents

One source, three outputs:

- `GUIDE.md` is the crate-level rustdoc (`#![doc = include_str!("../GUIDE.md")]`), so every example in it is a doctest.
- `llms.txt` (index) and `llms-full.txt` (guide + agent rules) are generated from `GUIDE.md` and `AGENTS.md` by `tests/docs.rs`; the test fails when they are stale, `just docs` regenerates them.
- `AGENTS.md` holds the rules for agents that use or change the crate.

## Extension points

| What | Trait | Default |
| --- | --- | --- |
| Singleton/scoped cache | `Storage` + `SingletonCell` | `ThreadSafe` (`OnceLock`); `SingleThread` (`OnceCell`) for `no_std` or single-threaded apps |
| Lifecycle and diagnostics | `Hooks` (`on_create`, `on_resolve`) | `NoHooks`, compiles away; `TracingHooks` behind the `tracing` feature |
| Introspection | `Container` (`NAME`, `PROVIDERS`, `describe()`) | generated |
| Construction | `Injectable` | derive or `#[injectable]` |
| Resolution | `Provide<T>` | generated; implement it for a hand-written container |

## Research & benchmarks

Condensed from the research report of 2026-10-08 (full report outside this repository). Registry facts come from the crates.io API on that date; anything taken only from a README, blog or issue says so.

### Plan that was followed

1. Find the existing DI design in the workspace (read-only). Result: no Rust design; a Dart design (`research/dart-di-design/`, runtime and codegen tracks) whose model (container, scopes, lifetimes, test overrides, graph checks) injecta moves into the Rust type system.
2. Read the repo conventions (toolchain pin, lints, MSRV) and install nothing new.
3. Research the Rust DI crates: crates.io API, READMEs, sources, published benchmarks, forum threads and issues.
4. Pick a name free on crates.io (core, `-macros`, `_macros`) and free locally.
5. Design core traits + proc macros; implement with tests, docs, an example and a benchmark workspace.
6. Verify: fmt, clippy `-D warnings` (pedantic), tests, doctests, compile-fail suite, MSRV 1.86.
7. Measure the same graph in injecta, nject, teloc, shaku, dill and hand-written code: resolve time, build time, binary size, lines.
8. Compare with DI in other ecosystems and map reported pain points to the design.

### Name check (crates.io API, 404 = free)

| Name | Result |
| --- | --- |
| `injectum` | taken (715 downloads) |
| `injectum-macros`, `injectum_macros` | free |
| **`injecta`, `injecta-macros`, `injecta_macros`** | **free, chosen** |
| `injix`, `injix-macros`, `injix_macros` | free |
| `injecto`, `injecto-macros`, `injecto_macros` | free |

### Local measurements

Same 7-type graph in every implementation (sources in `bench/`). Apple M3 Max (16 cores, 64 GB), macOS 27.0.1, rustc 1.99.0, `--release` (opt-level 3, no LTO), 2026-10-08. Resolve: divan 0.1.21, 500 samples, medians of two runs. Build times: `bench/measure.sh`, run 3 (5 crate-only samples); the machine was heavily loaded (load average 280–375), so wall times varied up to 3x between runs and only the coarse picture holds. Binary sizes are exact. Lines = non-blank, non-comment lines of each `impl-*/src/lib.rs` (types + DI annotations + wiring) / of the wiring section only.

| Implementation | `Handler` resolve | warm singleton | stripped binary | cold build incl. deps, wall / CPU | crate-only build, wall / CPU | touch rebuild | lines total / wiring |
| --- | ---: | ---: | ---: | --- | --- | ---: | ---: |
| hand-written | 8.5 ns | 3.6–4.0 ns | 343,144 B | 0.61 s / 0.55 s | 0.41 / 0.37 s | 0.33 s | 104 / 46 |
| **injecta** | 9.1–9.5 ns | 3.8–4.0 ns | 343,096 B | 4.69 s / 4.15 s | 0.68 / 0.46 s | 2.97 s* | 97 / 23 |
| nject 0.5.1 | 8.2–8.6 ns | 3.8–4.0 ns | 343,120 B | 4.07 s / 3.81 s | 0.37 / 0.35 s | 0.38 s | 99 / 37 |
| teloc 0.2.0 | 10.3 ns | 3.8–4.0 ns | 360,048 B | 5.23 s / 9.35 s | 0.56 / 0.43 s | 0.41 s | 106 / 17 |
| shaku 0.6.3 | 68–69 ns | 3.9–4.1 ns | 378,144 B | 5.30 s / 5.39 s | 0.85 / 0.80 s | 0.98 s | 165 / 22 |
| dill 0.17.1 | 494–515 ns | 46 ns | 431,056 B | 4.70 s / 10.25 s | 0.87 / 1.36 s | 0.88 s | 97 / 21 |

\* one sample under load; the two earlier runs measured 0.52 s and 1.78 s.

Boilerplate, before and after. Hand-written wiring (46 lines) builds every value and repeats every shared clone at each construction site:

```text
let repo = UserRepo { db: app.db.clone(), logger: app.logger.clone() };
let service = UserService { repo, cache: app.cache.clone(), logger: app.logger.clone() };
Handler { service, config: app.config.clone() }
```

injecta (23 lines of wiring plus one `#[derive(Injectable)]` per struct); adding a dependency is one field:

```text
injecta::container! {
    pub struct App {
        instance Config,
        singleton Arc<dyn Logger> = |c| Arc::new(c.build::<ConsoleLogger>()),
        singleton Arc<Database>,
        singleton Arc<Cache>,
        transient UserRepo,
        transient UserService,
        transient Handler,
    }
}
let handler = app.resolve::<Handler>();
```

shaku needs a trait, a `Component`/`Provider` impl and accessors per service (165 lines in total for the same graph).

### Published benchmarks

| Source | Numbers |
| --- | --- |
| [nject `examples/benchmark`](https://github.com/nicolascotton/nject/tree/main/examples/benchmark) (self-reported, nightly `cargo bench`) | baseline `by_ref` 10,783 ns/iter vs nject 10,721; `by_value` 158,883 vs 139,292: nject ≈ hand-written |
| [dependency-injector RUST_DI_COMPARISON.md](https://github.com/pegasusheavy/dependency-injector/blob/HEAD/RUST_DI_COMPARISON.md) (self-reported, Dec 2025, Rust 1.85) | singleton: shaku 17–21 ns, dependency-injector 18–24 ns, manual 21–23 ns, ferrous-di 57–70 ns, HashMap+RwLock 60–73 ns, DashMap 84–123 ns; 4-level chain: shaku 16–17, manual 17–19, ferrous-di 49–53 ns |
| [ferrous-di README](https://github.com/s1ntropy/ferrous-di) (self-reported) | singleton hit ~78 ns, cold ~437 ns, scoped ~83 ns, transient ~68 ns, scope create/drop ~18 ns |
| shaku, teloc, dill, syrette, coi, minfac, springtime, entrait | none published (shaku [#17](https://github.com/AzureMarker/shaku/issues/17) asks for them) |

No crate publishes compile-time or binary-size numbers.

### Feature matrix

| | Checked | Lifetimes | Scopes | Async | `dyn` required | Foreign types | Test overrides | Errors |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **injecta** | compile time | instance, lazy singleton, scoped, transient | yes, captive check at compile time | no (build first, `instance`) | no | factory | `.with(fake)` | compile errors with fix hints |
| [shaku](https://github.com/AzureMarker/shaku) | compile time | singleton (component), transient (provider) | no | no | yes (`Interface`) | hard | `with_component_override` | compile errors; provider `Result` |
| [nject](https://github.com/nicolascotton/nject) | compile time | provider fields, by value, references | `#[scope]` | no | no | `#[provide]` | none | compile errors |
| [teloc](https://github.com/p0lunin/teloc) | compile time | transient, lazy singleton, instance | `fork()` | no | no (`Arc<dyn>` breaks, #38) | conversions | fork + instance | large HList errors |
| [dill](https://github.com/kamu-data/dill-rs) | runtime + `validate()` | transient, singleton, agnostic, transaction | chained catalogs | — | interfaces | builders | chained catalog | `Result` at runtime |
| [syrette](https://github.com/HampusMat/Syrette) | runtime | transient, singleton | — | `async` feature | interfaces | yes | rebind | `Result`, cycle detection |
| [minfac](https://github.com/mineichen/minfac) | runtime, validated at `build()` | transient, shared, instance | child providers | no | no | yes | child providers | `Result` at build |
| [entrait](https://github.com/audunhalland/entrait) | compile time | n/a (traits from functions) | n/a | yes | optional | — | mocks | compile errors |

"—" = not documented in the sources read.

### Wins and losses

Wins: the only crate here with compile-time missing-provider, cycle and captive-dependency errors together with lazy singletons, scopes and test overrides; resolve within about 1 ns of hand-written code (shaku 7x slower on transients, dill ~55x); binary size equal to hand-written; concrete types by default; error messages that say what to write; pluggable storage and hooks; `no_std + alloc`.

Losses: no async or fallible providers, no modules, no multi-bindings, no runtime registration; factories are opaque to the cycle check; the cycle error is rustc's long query-cycle text and the override error prints the type as `_`; the one-time `syn` build every macro crate pays; v0.1 with no users.

### Borrow and avoid

Borrow: nject's zero-cost resolve measured against a hand-written baseline; teloc's lifetimes and fork scopes (without its HList types); minfac/.NET fail-fast and captive-dependency checks (done at compile time); dill's graph export and `AllOf`/`Lazy` as compile-time multi-bindings and lazy wrappers; shaku-style submodules; Go fx lifecycle start/stop as an explicit async `ready` step; Swift Dependencies-style test overrides.

Avoid: a trait per service and `Box<dyn>` per resolve; unnameable provider types; runtime `TypeId` maps on the hot path; auto-discovery, string names, runtime profiles, property injection and global locators; async inside `resolve`; nightly-only features; panicking resolves.

## Tasks

The prioritized task list (P0/P1/P2: publication prerequisites, CI wiring, factory-cycle guard, diagnostics, modules, async providers, lifecycle, multi-bindings, axum/tokio integration, docs) is [`todo.md`](todo.md); details of the numbered tasks are in [`plan.md`](plan.md).

## Development

`just check` runs fmt, clippy (`-D warnings`, pedantic, all targets and features, plus the `no_std` build), nextest, doctests, the compile-fail suite and the MSRV (1.86) check. See [`AGENTS.md`](AGENTS.md).

License: GPL-3.0-or-later.
