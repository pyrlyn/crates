# Research

T1: how lockstep makes deadlocks impossible, and what it reuses. Sources were checked on
2026-10-25; registry facts come from the crates.io API (`/api/v1/crates/<name>`), versions are
the newest stable release on that date.

## The problem

Safe Rust does not prevent deadlocks: the Reference lists them under behavior *not* considered
unsafe (https://doc.rust-lang.org/reference/behavior-not-considered-unsafe.html). Locking a
`std::sync::Mutex` twice on one thread is unspecified; the call "will not return" and "might
panic or deadlock" (https://doc.rust-lang.org/std/sync/struct.Mutex.html, Rust 1.99.1).

A deadlock needs mutual exclusion, no preemption, hold-and-wait and circular wait; a library can
attack the last two (happylock README, https://crates.io/crates/happylock 0.5.2).

## Rust options

| Crate | Version, last release | How it prevents or finds deadlocks | Notes |
| --- | --- | --- | --- |
| `lock_tree` | 0.1.0, 2025-01-09 | Compile time: marker type per level, `LockAfter` traits, a `Locked` token borrowed mutably on every acquisition; `impl_lock_after!` refuses an inverted pair | Republication of Fuchsia netstack3 `lock-order` (https://docs.rs/lock_tree/0.1.0, https://fuchsia.googlesource.com/fuchsia/+/refs/heads/main/src/connectivity/network/netstack3/core/lock-order/); 5 recent downloads |
| `lock_ordering` | 0.2.0, 2025-02-20 | Same design as `lock_tree`; `LockedAt` token, `impl_transitive_lock_order!`; docs admit nothing stops a cycle in hand-written trait impls | https://docs.rs/lock_ordering/0.2.0; repo pushed 2025-03-17; 43 recent downloads |
| `ordered-locks` | 0.2.1, 2022-12-16 | Compile time: fixed levels `L1`…, token split from a guard; root token is `unsafe` to create | https://crates.io/crates/ordered-locks; repo untouched since 2022-12-16, so dead |
| `happylock` | 0.5.2, 2026-08-27 | Compile time plus a run-time check: one `ThreadKey` per thread (taken through a thread-local check), a lock needs the key, several locks are taken at once as a `LockCollection` sorted by address | https://crates.io/crates/happylock; CC0-1.0; 122 recent downloads; no async (README "Future Work") |
| `tracing-mutex` | 0.3.3, 2026-05-20 | Run time: global lock-dependency graph, panics when an acquisition would close a cycle; debug-only aliases fall back to the plain lock in release | https://docs.rs/tracing-mutex/0.3.3; backtraces cost "over 60%"; wraps std, parking_lot, lock_api |
| `no_deadlocks` | 1.3.2, 2023-11-19 | Run time: global lock manager searches the wait-for graph when a lock blocks; "much slower" by its own README | https://crates.io/crates/no_deadlocks; MPL-2.0; repo untouched since 2023-11-19 |
| `parking_lot` | 0.13.0, 2026-10-21 | Run time, experimental `deadlock_detection` feature: `check_deadlock()` finds threads already deadlocked; reporting leaves them blocked | https://docs.rs/parking_lot/0.13.0/parking_lot/deadlock/; MSRV 1.95 |

The same ideas outside Rust: Linux lockdep validates the order between lock classes at run time
(https://docs.kernel.org/locking/lockdep-design.html); Abseil's `Mutex` detects inconsistent
order in non-opt builds (https://abseil.io/docs/cpp/guides/synchronization).

### Compile-time level comparison, measured

Two ways to say "level A is below level B" on stable Rust:

- **Const generic levels** (`OrderedMutex<const L: u32, T>`, `const { assert!(HELD < L) }`).
  A where-bound like `where [(); L - HELD]:` needs `generic_const_exprs`, which is unstable
  (https://github.com/rust-lang/rust/issues/76560, open). Inline `const` blocks are stable since
  1.79 (https://blog.rust-lang.org/2024/06/13/Rust-1.79.0/) and can read in-scope generics. A probe
  on rustc 1.99.1 showed the catch: an out-of-order lock passes `cargo check` (and so clippy and
  rust-analyzer) and fails only at `cargo build` with `E0080`, and only for code that is
  monomorphized.
- **Marker types with order traits** (`lock_tree`, `lock_ordering`). The error is an ordinary
  trait-bound error, reported by `cargo check` and the IDE. The risk is a cycle in hand-written
  impls; generating every impl from one ordered list removes it.

## Other languages

- **Go**: "Do not communicate by sharing memory; instead, share memory by communicating"
  (https://go.dev/doc/effective_go). Channels still deadlock; the runtime only catches the case
  where every goroutine is blocked (`fatal("all goroutines are asleep - deadlock!")` in
  `checkdead`, https://github.com/golang/go/blob/master/src/runtime/proc.go, master on
  2026-10-25).
- **Erlang**: processes "share no data" and talk by message passing
  (https://www.erlang.org/doc/system/conc_prog.html). Two processes calling each other still wait
  forever; OTP bounds it with a call timeout: `gen_server:call/2` is `call(ServerRef, Request,
  5000)` (https://www.erlang.org/doc/apps/stdlib/gen_server.html).
- **Haskell STM**: `atomically`, `retry`, `orElse` compose transactions without visible locks
  (https://hackage.haskell.org/package/stm/docs/Control-Monad-STM.html, stm 2.5.3.1; Harris et al.,
  "Composable Memory Transactions",
  https://www.microsoft.com/en-us/research/publication/composable-memory-transactions/). In Rust
  the STM crates are dead: `stm` 0.4.0 (2018-03-29), `swym` 0.1.0-preview (2019-02-25);
  `async-stm` 0.5.0 (2024-05-09) is small and async-only.

What transfers: message passing and STM avoid lock order rather than enforce it, and Rust
already has channels (`std::sync::mpsc`, crossbeam). The Erlang timeout is a recovery, not a
prevention. None of them is what lockstep is for.

## Lock-free structures

| Crate | Version, last release | Offers | Notes |
| --- | --- | --- | --- |
| `crossbeam-queue` | 0.3.14, 2026-09-05 | `ArrayQueue` (bounded), `SegQueue` (unbounded), MPMC | https://docs.rs/crossbeam-queue/0.3.14; MSRV 1.60; only dependency `crossbeam-utils`, already in this workspace's `Cargo.lock` (0.8.23); no stack |
| `scc` | 3.8.8, 2026-09-01 | `Stack` and `Queue` ("lock-free concurrent" LIFO and FIFO), `Bag`, hash maps | https://docs.rs/scc/3.8.8; Apache-2.0; MSRV 1.85; depends on `sdd`, `saa`; 20M recent downloads |
| `lockfree` | 0.5.1, 2018-11-18 | Queue, stack, map | Dead |

## Model testing

| Crate | Version, last release | Approach |
| --- | --- | --- |
| `loom` | 0.7.2, 2024-04-23 | Exhaustive permutation testing under the C11 memory model; code swaps in `loom::sync` behind `cfg(loom)` (https://docs.rs/loom/0.7.2); repo pushed 2026-02-20 |
| `shuttle` | 0.9.8, 2026-10-20 | Randomized scheduling, scales to larger tests (https://crates.io/crates/shuttle) |

## Decision

1. **Global lock order, enforced at compile time; lock-free types beside it.** Both, as the
   crate already describes: the order covers code that needs locks, lock-free types cover code
   that does not.
2. **Levels are marker types with order traits, generated from one ordered list** by a
   `macro_rules!` macro (`levels!(Config, Session, Cache)` implements "after" for every later
   item). Errors come from `cargo check`, as with `lock_tree`; a single list makes a cycle
   impossible, which `lock_ordering`'s hand-written impls do not. Const generic levels are
   rejected: errors only at `cargo build`.
3. **Acquisition goes through a token** borrowed mutably (the `lock_tree` / `lock_ordering`
   pattern): locking a level-`B` lock needs `&mut Token<A>` with `B` after `A` and returns the
   guard plus `Token<B>`. The root token comes from a thread-local check, one live root per thread
   (happylock's `ThreadKey`), so a nested function cannot mint a fresh token while locks are held.
   Both pieces are safe code.
4. **Locks of one level are taken together**, through a collection that locks them in address
   order (happylock's `LockCollection` idea). This is what the dining-philosophers case needs.
5. **Backend: `std::sync::Mutex` and `RwLock`.** No dependency, futex-based on Linux since 1.62
   (https://blog.rust-lang.org/2022/06/30/Rust-1.62.0/). `parking_lot` 0.13 needs Rust 1.95, above
   this crate's `rust-version = "1.86"`, and is not needed.
6. **No crate is wrapped for ordering.** `lock_tree` and `lock_ordering` have almost no users and
   no release in over a year, `ordered-locks` is dead, happylock's key model does not take
   levels. The design is small enough to own; the sources above are credited in the docs.
7. **Run-time detector (T6)** covers what types cannot: locks whose level is known only at run
   time (for example one lock per record, ordered by id). A thread-local stack of held ranks,
   on in debug builds, a feature flag in release, panic or log. Our levels make it a rank
   comparison, so `tracing-mutex`'s graph is not needed.
8. **Lock-free (T4): `scc` for `Stack` and `Queue`**, one dependency for both and within the
   MSRV; T4 adds `crossbeam-queue` only if a bounded queue is needed. `unsafe_code = "forbid"`
   stays: the unsafe code lives in the reused crate.
9. **Model tests (T7): `loom`**, the reference tool for small models; `shuttle` only if loom
   cannot explore a test.
10. **Out of scope for now: async.** A per-thread token does not survive tasks that move between
    threads (happylock README names this as its main open problem). Recorded in `ideas.md`.
