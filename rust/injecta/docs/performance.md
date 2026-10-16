# Performance

What injecta costs at run time, what was optimized, the full criterion results and the ideas that are left. Every number here was measured; none is estimated.

**Machine and conditions.** Ivan's MacBook Pro: Apple M3 Max (16 cores: 12 performance + 4 efficiency), 64 GB, macOS 27.0.1; rustc 1.99.0; `--release` (opt-level 3, no LTO, `codegen-units` default); criterion 0.8.2 (500 ms warm-up, 2 s measurement, 100 samples, 20 for the thread groups); 2026-10-08, 15:47–15:53 EEST. **The machine was shared with other build jobs**: load average 58 when the criterion run started and 468 when it ended. Expect a few percent of noise on single-digit nanosecond numbers and much more on the thread-scaling groups; compare numbers within one table (same run), not across runs. Raw output: [`bench/results/criterion-2026-10-08.md`](../bench/results/criterion-2026-10-08.md).

Reproduce from `bench/`: `cargo bench -p criterion-bench --bench criterion` (or `just bench-criterion`), then `python3 criterion-bench/report.py` for the table. Benchmarks are a separate, unpublished workspace, so CI and `cargo test` never build them.

## What a resolve compiles to

For the 7-type comparison graph (`bench/impl-injecta`), `app.resolve::<Handler>()` is fully inlined into straight-line code: per shared entry one acquire load of the `OnceLock` state, a branch to an out-of-line cold initializer, and the `Arc` reference-count increment that the hand-written version also does. No locks, no `dyn` calls of injecta's own, no maps, no allocation beyond the user's types. On aarch64 that is `ldapur` + `cbnz` + `ldadd` per singleton. The hand-written baseline builds singletons eagerly and so skips the load and branch; that is the whole difference in the `transient_graph` rows below.

## Optimizations in this round

| Change | Before | After | Verdict |
| --- | --- | --- | --- |
| **`Resolve::get::<T>()` / `ProvideRef<T>`**: borrow a cached entry (`instance`, `singleton`, `scoped`) instead of cloning it | warm `Arc<Database>` via `resolve`: 5.65 ns (`ThreadSafe`), 5.52 ns (`SingleThread`) | via `get`: 0.756 ns, 0.673 ns | **7.5–8.2x** per access; kept |
| same, 1 000 accesses in a loop | `resolve`: 5.27 µs (190 M/s) | `get`: 0.628 µs (1 593 M/s) | **8.4x**; kept |
| same, one shared singleton read from 8 threads | `resolve`: 21.2 M/s in total (hand-written `Arc::clone`: 21.2 M/s) | `get`: 1 048 M/s | **49x**: no reference-count cache line bouncing between cores; kept |
| same, 4 threads | `resolve`: 34.0 M/s | `get`: 992 M/s | **29x**; kept |
| same, scoped value in a request scope | `resolve`: 5.21 ns | `get`: 0.733 ns | **7.1x**; kept |
| Cached entries' `provide` became `clone(provide_ref())`, so the lazy-init code is generated once per entry | `resolve_handler` / `resolve_singleton` / `cold`: 137 / 36 / 29 aarch64 instructions; stripped binary 343 096 B | 137 / 36 / 29 instructions, identical apart from symbol names; 343 096 B | no run-time or size change; kept because `get` needs the borrow path anyway and it removes a duplicate code path |
| `#[allow(clippy::clone_on_copy)]` on the generated `provide` | `instance RequestId` (a `Copy` type) made the user's `clippy -D warnings` fail | clean | lint hygiene, no run-time effect; kept |

Crate-only release build of `bench/impl-injecta` (5 runs each, CPU seconds, same machine): before 0.57–0.69 s (median 0.65), after 0.53–0.66 s (median 0.58): no measurable compile-time change from the extra `ProvideRef` impls. The divan resolve suite before the change (load average 36–61): injecta `transient_graph` 10.4 ns median, `singleton` 4.42 ns, `cold` 102.5 ns. The run after the change hit load average 56–130 and measured 32.9 / 4.38 / 132.4 ns while the baseline stayed at 9.3 ns; the machine code of those functions is identical before and after (table above), so that run was scheduled badly (most likely on an efficiency core) rather than slower. Raw files: `bench/results/perf-before.txt`, `perf-after.txt`.

Considered and not attempted: `#[inline(always)]` on generated bodies (the assembly shows everything on the hot path is already inlined, so there is nothing to gain); a cheaper thread-safe cell than `OnceLock` (the faster designs need `unsafe`, which the crate forbids, or a new dependency; listed as P1-3 below).

## Results

### Comparison with other DI crates (same graph)

| | hand-written | **injecta** | nject 0.5.1 | teloc 0.2.0 | shaku 0.6.3 | dill 0.17.1 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| resolve `Handler` (3 transients, 4 shared values) | 11.0 ns | 11.9 ns | 11.2 ns | 26.7 ns | 217 ns | 628 ns |
| warm singleton | 5.39 ns | 5.34 ns | 7.83 ns | 7.07 ns | 7.99 ns | 56.5 ns |
| build container + first `Handler` + drop | 105 ns | 102 ns | 172 ns | 207 ns | 1.63 µs | 2.69 µs |

These rows ran while the load climbed; the divan suite in the README (500 samples, two runs) is the steadier comparison; both put hand-written, injecta and nject within about 1 ns of each other on the transient graph, and shaku and dill an order of magnitude or more behind.

### Cold start

| | `ThreadSafe` (default) | `SingleThread` | hand-written |
| --- | ---: | ---: | ---: |
| `App::new(config)` (no value built yet) | 3.91 ns | 3.04 ns | — |
| `App::new` + first `Handler` (builds 3 singletons) | 103 ns | 89.8 ns | 85.7 ns (eager `new_app`) |
| first resolve of `Arc<Database>` (creates it and the `Logger` singleton it needs) | 48.6 ns | 37.7 ns | — |

`SingleThread` skips the `Once` state machine on first initialization (about 11 ns for the two singletons the first `Database` resolve creates) and is the right choice for CLIs, `current_thread` runtimes and `no_std`.

### Singletons: lazy, cached, borrowed

| | `ThreadSafe` | `SingleThread` |
| --- | ---: | ---: |
| first resolve (lazy init) | 48.6 ns | 37.7 ns |
| cached `resolve` (clone the `Arc`) | 5.65 ns | 5.52 ns |
| cached `get` (borrow) | 0.756 ns | 0.673 ns |

### Transients

| | time |
| --- | ---: |
| `resolve::<Handler>()`, `ThreadSafe` | 14.1 ns |
| `resolve::<Handler>()`, `SingleThread` | 14.4 ns |
| `resolve::<Handler>().checksum()` | 12.6 ns |
| hand-written, same checksum | 12.2 ns |

### Deep dependency chains

A chain of transients `L1 -> L2 -> … -> Ln` (generated by `bench/criterion-bench/gen_chains.py`), resolved through injecta and built by hand:

| depth | injecta | by hand |
| ---: | ---: | ---: |
| 10 | 0.582 ns | 0.574 ns |
| 20 | 1.16 ns | 1.24 ns |
| 50 | 1.33 ns | 1.17 ns |

Both versions compile to the same few instructions (the chain is plain data, so the optimizer folds it), which is the point: depth costs nothing at run time. The compile-time depth check accepts chains up to `MAX_DEPTH` (64).

### Throughput in a loop (1 000 operations per iteration)

| | time | throughput |
| --- | ---: | ---: |
| injecta `resolve::<Handler>()` | 12.7 µs | 79.0 M/s |
| hand-written `Handler` | 12.0 µs | 83.4 M/s |
| injecta singleton `resolve` | 5.27 µs | 190 M/s |
| injecta singleton `get` | 0.628 µs | 1 593 M/s |

### Threads (total operations per second, 10 000 per thread)

| threads | singleton `resolve` | singleton `get` | singleton by hand (`Arc::clone`) | `Handler` `resolve` | `Handler` by hand |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 175 M/s | 589 M/s | 160 M/s | 69.0 M/s | 82.4 M/s |
| 2 | 75.6 M/s | 587 M/s | 71.2 M/s | 14.6 M/s | 15.7 M/s |
| 4 | 34.0 M/s | 992 M/s | 32.2 M/s | 6.6 M/s | 6.7 M/s |
| 8 | 21.2 M/s | 1 048 M/s | 21.2 M/s | 4.2 M/s | 5.5 M/s |

Anything that clones a shared `Arc` gets slower with more threads, injecta or not: every clone and drop writes the same reference-count cache line. `get` only reads, so it scales. The 1-thread `get` row includes thread start-up (17 µs per 10 000 operations); with more threads it reaches about 1 G/s.

### Scopes

| | time |
| --- | ---: |
| open a scope, resolve a `scoped Arc<Session>` (built once), drop | 32.9 ns |
| open a scope, resolve a handler that uses scoped and root values, drop | 40.3 ns |
| cached scoped value, `resolve` | 5.21 ns |
| cached scoped value, `get` | 0.733 ns |
| root singleton through a scope | 5.54 ns |
| root transient rebuilt in a scope | 11.5 ns |

### Overrides

| | time |
| --- | ---: |
| `App::new(config)` | 3.70 ns |
| `App::new(config).with(fake_logger)` | 5.56 ns |
| the same plus the first `Handler` resolve | 81.0 ns |

`.with` costs about 2 ns (one `Arc` clone of the fake and a preset cell) and saves the real constructor.

## Speedup ideas, prioritized

| # | Idea | Expected impact | Effort | Notes |
| --- | --- | --- | --- | --- |
| P0-1 | Document `get` as the default for reading shared values in hot paths and handlers; lint-like note in `AGENTS.md` | 7–50x on those call sites (measured above) | S | pure docs |
| P0-2 | Recommend `SingleThread` for single-threaded programs in the guide's decision table (done) and in examples | ~20% on first initialization, ~1 ns on construction | S | done in `GUIDE.md` |
| P0-3 | `App::warm()`: initialize every singleton up front (opt-in), so the first request does not pay 40–100 ns per singleton plus the constructors | removes first-request latency spikes; no hot-path change | S | keeps lazy as the default |
| P1-1 | Borrowing injection: let `#[derive(Injectable)]` take `&'c T` fields for transients built from a container, so transient graphs stop cloning `Arc`s | removes 4 atomic increments and decrements per `Handler`; the thread table suggests multi-x under contention | L | needs a lifetime on `Injectable` and the container; design work |
| P1-2 | Owned scopes for async servers (`Request<Arc<App>>`) without an extra `Arc` clone per request | saves one atomic pair per request | M | prerequisite for axum integration |
| P1-3 | Racy lock-free init for `ThreadSafe` cells (`once_cell::race::OnceBox`-style) | about 5 ns per singleton on first init (half the measured `ThreadSafe` vs `SingleThread` gap for two singletons) | M | needs a dependency or `unsafe`; first init only |
| P2-1 | Skip `Container`/`Debug` generation behind a feature for containers that do not need introspection | smaller generated code; compile time only | S | measure before keeping |
| P2-2 | Parse factories as token streams and drop `syn/full` where possible | less proc-macro build time on clean builds, if no other crate in the build already enables `syn/full` (most do) | L | unlikely to matter in real projects |
| P2-3 | Generate per-container `resolve_*` functions for the hottest entries | none expected: the generic path already inlines completely | S | only with a benchmark that shows a gain |
