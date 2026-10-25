# lockstep — plan

GitHub: https://github.com/pyrlyn/crates/tree/main/rust/lockstep

Locks that cannot deadlock: one global acquisition order, or lock-free structures. Research and
the chosen approach (decisions 1–10): [docs/research.md](docs/research.md). Finished tasks:
[done.md](done.md).

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P1 | 4 | 0% | |
| T3 | todo | P1 | 3 | 0% | |
| T4 | todo | P2 | 2 | 0% | |
| T5 | todo | P2 | 2 | 0% | |
| T6 | todo | P1 | 3 | 0% | |
| T7 | todo | P2 | 3 | 0% | |
| T8 | todo | P2 | 3 | 0% | |
| T9 | todo | P3 | 2 | 0% | |

### T2. Lock levels and an acquisition API with a global order

Every lock has a level; a thread may only take a lock above the level it holds (research
decisions 2 and 3).

Done: in `src/ordered/`, a `macro_rules!` macro that turns one ordered list of marker types into
levels and the "after" relation for every pair; a `Token<L>` whose root comes from a thread-local
check (one live root per thread, `None` otherwise) and is not `Send`; an acquisition API that
takes `&mut Token<A>` and returns the next token only when the lock's level is after `A`.
Out-of-order locking is a `cargo check` error, proved by compile-fail tests (trybuild or doctests
marked `compile_fail`); a second root token on the same thread is refused at run time.

### T3. Mutex and RwLock wrappers with order checking

`OrderedMutex` and `OrderedRwLock` over `std::sync::Mutex` and `RwLock` (decision 5), with their
guards, on the T2 API; plus a collection that takes several locks of one level at once in address
order (decision 4).

Done: the wrappers lock and unlock and give the token back when the guard drops; read and write
guards of `OrderedRwLock` follow the same order; poisoning behaves as in `std`; the collection
locks every member or none and never twice. Tests cover nested acquisition, guard drop order and
the collection.

### T4. Lock-free queue and stack

Where no lock is needed, a lock-free queue or stack removes the problem. They wrap `scc::Queue`
and `scc::Stack` (decision 8), so `unsafe_code = "forbid"` stays.

Done: queue and stack types in `src/lockfree/` with tests under concurrent producers and
consumers; `scc` is listed in `toolchain.md` and `rust.md`.

### T5. Detect locks taken outside the crate

The guarantee holds only if all locking goes through lockstep.

Done: a shipped clippy `disallowed-types` config that rejects `std::sync::Mutex`, `RwLock` and
the `parking_lot` locks, documented in the README with how a consumer enables it; a test or
example proves clippy reports such a lock.

### T6. Runtime order-violation detector

For locks whose rank is known only at run time, such as one lock per record ordered by id
(decision 7). Fires when a thread takes a rank not above every rank it holds. On by default in
debug builds, behind a feature flag in release; configurable to panic or to log.

Done: the detector in `src/detect.rs` with a thread-local stack of held ranks, the feature flag
and the panic/log switch; tests for both modes and for the release default.

### T7. Model and stress tests

Prove the order holds under every interleaving loom explores and under load (decision 9).

Done: loom model tests for the ordered locks and the root-token check behind a `loom` cfg, a
stress test with many threads, both run in CI.

### T8. Deadlock scenario tests

Classic cases that deadlock without lockstep: AB/BA with two mutexes, dining philosophers, a
lock cycle across three threads, RwLock upgrade.

Done: each scenario runs with `std` locks under a timeout and is shown to hang, then the same
scenario with lockstep is shown impossible: a compile-fail test for the ordered levels, the
same-level collection for the philosophers, a runtime rejection for run-time ranks.

### T9. Documentation with examples

Done: crate docs and the README describe enabling, disabling, testing and the limits, with
runnable examples for the ordered locks, the collection, the lock-free types and the detector,
and credit `lock_tree`, `lock_ordering` and happylock for the ideas taken from them; "Skeleton
only" is gone from the README and `lib.rs`.
