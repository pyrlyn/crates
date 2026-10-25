//! Ordered locks.
//!
//! Intended contents: lock levels and the `OrderedMutex` and `OrderedRwLock`
//! wrappers over the `std::sync` locks. Levels are marker types whose order a
//! macro generates from one list; acquiring a lock borrows a token for a lower
//! level, so out-of-order locking is a `cargo check` error. Const generic
//! levels were rejected because their check fails only at `cargo build` (see
//! `docs/research.md`).
