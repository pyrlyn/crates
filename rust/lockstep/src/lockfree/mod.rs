//! Lock-free structures.
//!
//! Intended contents: a lock-free MPMC queue and a lock-free stack, for
//! cases where shared state can be exchanged without holding any lock. Both
//! wrap `scc`, because this crate forbids `unsafe` (see `docs/research.md`).
