# Done

### T0. Core crate, macros, tests, docs pipeline, comparison benchmarks

`injecta` (traits, storage, hooks, introspection) and `injecta-macros` (`container!`, `#[derive(Injectable)]`, `#[injectable]`), compile-fail tests with pinned messages, `GUIDE.md` as the single docs source for rustdoc, `llms.txt` and `llms-full.txt`, and the `bench/` workspace comparing injecta with shaku, nject, teloc, dill and hand-written wiring.

### T5. Borrow fast path, criterion suite, more tests

`ProvideRef<T>` and `Resolve::get` lend cached entries (`instance`, `singleton`, `scoped`) without cloning; cached `provide` is now a clone of `provide_ref`, so the lazy-init code is emitted once per entry. Generated `provide` allows `clippy::clone_on_copy` (it fired on `Copy` instances). `#[injectable]` no longer adds a "cannot find attribute `inject`" error after its own. New tests: `tests/fast_paths.rs` (lazy init once for both storages and under 8-thread contention, `get`, fresh transients, per-scope values and drop, overrides, constructor injection) and four compile-fail cases (`get` on a transient, `scoped` in the root, `SingleThread` across threads, async constructor). Criterion suite in `bench/criterion-bench`; results and speedup ideas in `docs/performance.md`.
