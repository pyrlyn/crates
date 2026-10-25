# lockstep — plan

GitHub: https://github.com/pyrlyn/crates/tree/main/rust/lockstep

Locks that cannot deadlock: one global acquisition order, or lock-free structures. Research and
the chosen approach: [docs/research.md](docs/research.md). Finished tasks: [done.md](done.md).

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T1 | in progress | P0 | 3 | 10% | Claude Code / claude-opus-5-5 |
| T2 | todo | P1 | 4 | 0% | |
| T3 | todo | P1 | 3 | 0% | |
| T4 | todo | P2 | 4 | 0% | |
| T5 | todo | P2 | 2 | 0% | |
| T6 | todo | P1 | 3 | 0% | |
| T7 | todo | P2 | 3 | 0% | |
| T8 | todo | P2 | 3 | 0% | |
| T9 | todo | P3 | 2 | 0% | |

### T1. Research and choose the approach

Every later task depends on how the order is expressed and checked, so the choice comes first.
Study the Rust options: `parking_lot`'s `deadlock_detection`, what the borrow checker can and
cannot guarantee about locks, lock-ordering crates (`lock_ordering`, `happylock` and any
others), lock-free libraries such as crossbeam, and loom for model testing. Study other
languages: Go channels, Erlang actors and message passing, Haskell STM.

Done: `docs/research.md` compares the options with primary sources (URL plus version or date
checked) and records the decision: global lock order, lock-free, or both; how a level is
expressed (typestate token, const generic, runtime rank); which existing crates lockstep wraps or
reuses instead of reimplementing; how the crate's `unsafe_code = "forbid"` fits the lock-free
part. Tasks T2–T9 are updated to match the decision.

Execution plan:

1. Read the docs and source of `parking_lot`, `happylock`, `lock_ordering` and similar crates
   found on crates.io, crossbeam, loom; note version and maintenance state from the registry.
2. Read the primary docs on Go channels, Erlang message passing and Haskell STM; note only what
   transfers to a Rust library.
3. Write `docs/research.md`: option table, reuse candidates, decision and its reasons.
4. Adjust the T2–T9 cards and `toolchain.md` to the decision.
5. Check: `cargo test -p lockstep`, `cargo clippy -p lockstep --all-targets -- -D warnings`,
   `cargo fmt --all -- --check` still pass; every fact in `docs/research.md` has a source.

### T2. Lock levels and an acquisition API with a global order

Every lock has a level; a thread may only take a lock above every level it holds. Checked at
compile time (typestate or const generics) where the type system allows it, at run time in debug
builds otherwise.

Done: a public level type and acquisition API in `src/ordered/`; taking a lower or equal level
while holding a higher one is a compile error or a debug-build rejection; unit tests for both.

### T3. Mutex and RwLock wrappers with order checking

`OrderedMutex`, `OrderedRwLock` and their guards on top of the T2 API.

Done: the wrappers lock, unlock and release their level on guard drop; read and write guards of
`OrderedRwLock` follow the same order; tests cover nested acquisition, out-of-order rejection and
guard drop order.

### T4. Lock-free queue and stack

Where no lock is needed, a lock-free queue or stack removes the problem. The crate forbids
`unsafe`, so the structures most likely wrap a maintained crate chosen in T1 instead of being
written here.

Done: queue and stack types in `src/lockfree/` with tests under concurrent producers and
consumers; the reused crate is listed in `toolchain.md` and `rust.md`.

### T5. Detect locks taken outside the crate

The guarantee holds only if all locking goes through lockstep.

Done: a shipped clippy `disallowed-types` config that rejects `std::sync::Mutex`, `RwLock` and
the `parking_lot` locks, documented in the README with how a consumer enables it; a test or
example proves clippy reports such a lock.

### T6. Runtime order-violation detector

Fires when acquisition order is violated at run time. On by default in debug builds, behind a
feature flag in release; configurable to panic or to log.

Done: the detector in `src/detect.rs` with the feature flag and the panic/log switch; tests for
both modes and for the release default.

### T7. Model and stress tests

Prove the order holds under every interleaving loom explores and under load.

Done: loom model tests for the ordered locks behind a `loom` cfg, a stress test with many
threads, both run in CI.

### T8. Deadlock scenario tests

Classic cases that deadlock without lockstep: AB/BA with two mutexes, dining philosophers, a
lock cycle across three threads, RwLock upgrade.

Done: each scenario runs with `std` or `parking_lot` under a timeout and is shown to hang, then
the same scenario with lockstep is shown impossible (a compile-fail test or a runtime
rejection).

### T9. Documentation with examples

Done: crate docs and the README describe enabling, disabling, testing and the limits, with
runnable examples for the ordered locks, the lock-free types and the detector; "Skeleton only"
is gone from the README and `lib.rs`.
