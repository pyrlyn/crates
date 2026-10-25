//! Ordered locks.
//!
//! Intended contents: lock levels (ranks) and the `OrderedMutex` and
//! `OrderedRwLock` wrappers. Acquiring a lock requires proof that the thread
//! holds only lower levels: a typestate token or const-generic level at
//! compile time where possible, a runtime check in debug builds otherwise.
