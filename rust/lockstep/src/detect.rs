//! Detection of order violations and of locks outside the crate.
//!
//! Intended contents:
//!
//! - **Runtime order-violation detector.** For locks whose rank is known only
//!   at run time, where the compile-time levels cannot help. Fires when a
//!   thread acquires a rank not above every rank it holds, reporting both
//!   locks and the thread. On by default in debug builds, behind a feature
//!   flag in release; configurable to panic or to log.
//! - **Locks outside the crate.** A clippy `disallowed-types` config that
//!   bans `std::sync::Mutex`, `std::sync::RwLock` and `parking_lot` locks in
//!   crates that adopt Lockstep, plus debug-mode warnings where possible.
