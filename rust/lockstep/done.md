# lockstep — completed tasks

### T1. Research and choose the approach

`docs/research.md` compares the Rust crates (`lock_tree`, `lock_ordering`, `ordered-locks`,
happylock, `tracing-mutex`, `no_deadlocks`, `parking_lot`'s deadlock detection), Go channels,
Erlang message passing and Haskell STM, the lock-free crates and loom/shuttle, with registry
versions and primary sources checked on 2026-10-25. Decision: a global lock order checked at
compile time through marker-type levels generated from one ordered list and a mutably borrowed
token (root from a thread-local check), same-level locks taken together in address order, over
`std::sync` locks; a run-time rank detector only for run-time ranks; `scc` for the lock-free
queue and stack; loom for model tests; async out of scope. A probe on rustc 1.99.1 showed const
generic levels fail only at `cargo build`, not `cargo check`, which ruled them out. Cards T2–T9
and the module docs were updated to match. Checked with `cargo test -p lockstep`, clippy with
`-D warnings` and `cargo fmt --check`.
