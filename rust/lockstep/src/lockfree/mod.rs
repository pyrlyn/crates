//! Lock-free structures.
//!
//! Intended contents: a lock-free MPMC queue and a lock-free stack, for
//! cases where shared state can be exchanged without holding any lock.
//! Whether to build these or wrap an existing crate (crossbeam) is decided
//! in the research stage.
