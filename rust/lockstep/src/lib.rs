//! Locks that cannot deadlock.
//!
//! Lockstep aims to make deadlocks impossible in two ways:
//!
//! - **One global lock order.** Every lock has a level. A thread may only
//!   acquire a lock whose level is above every level it already holds. With
//!   a single order no cycle of waiting threads can form. The order is
//!   checked at compile time where the type system allows it and at run time
//!   in debug builds.
//! - **Lock-free structures.** Where a lock is not needed at all, a
//!   lock-free queue or stack removes the problem entirely.
//!
//! The guarantee holds only if **all** locking goes through this crate. A
//! single `std::sync::Mutex` or `parking_lot` lock taken outside it is
//! invisible to the order, and the order can break. See [`detect`] for how
//! such locks are meant to be found.
//!
//! Status: skeleton. Modules hold only their intended design; see `plan.md`.

pub mod detect;
pub mod guard;
pub mod lockfree;
pub mod ordered;

#[cfg(test)]
mod testing;
