//! Guards.
//!
//! Intended contents: the guards returned by ordered locks, and per-thread
//! tracking of held levels. Dropping a guard releases its level; guards are
//! not `Send`, so a lock is always released on the thread that took it.
