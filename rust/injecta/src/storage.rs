//! Where singleton and scoped values are cached. Kept behind the `Storage`
//! trait so a container can swap the thread-safe default for a cheaper
//! single-threaded cell, or for a custom one, without touching generated code.

use core::cell::OnceCell;

/// A write-once cell that caches one singleton or scoped value.
///
/// Implement it (with [`Storage`]) to plug in a custom cache, for example one
/// that records initialization order.
pub trait SingletonCell<T> {
    /// An empty cell; the value is created on first resolve.
    fn empty() -> Self;
    /// A cell that already holds `value` (used by overrides).
    fn preset(value: T) -> Self;
    /// Returns the cached value, creating it with `init` on first use.
    fn get_or_init(&self, init: impl FnOnce() -> T) -> &T;
    /// Returns the cached value if it has been created.
    fn get(&self) -> Option<&T>;
}

/// Chooses the cell type for every cached entry of a container:
/// `container! { #[injecta(storage = SingleThread)] pub struct App { .. } }`.
pub trait Storage {
    /// The cell holding one cached value of type `T`.
    type Cell<T>: SingletonCell<T>;
}

/// `OnceLock` cells: containers are `Send + Sync` when their values are.
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy)]
pub enum ThreadSafe {}

#[cfg(feature = "std")]
impl Storage for ThreadSafe {
    type Cell<T> = std::sync::OnceLock<T>;
}

/// `OnceCell` cells: no atomics, but the container is not `Sync`.
#[derive(Debug, Clone, Copy)]
pub enum SingleThread {}

impl Storage for SingleThread {
    type Cell<T> = OnceCell<T>;
}

/// The storage `container!` uses when none is named: [`ThreadSafe`] with the
/// `std` feature, [`SingleThread`] without it.
#[cfg(feature = "std")]
pub type DefaultStorage = ThreadSafe;
/// The storage `container!` uses when none is named: [`ThreadSafe`] with the
/// `std` feature, [`SingleThread`] without it.
#[cfg(not(feature = "std"))]
pub type DefaultStorage = SingleThread;

#[cfg(feature = "std")]
impl<T> SingletonCell<T> for std::sync::OnceLock<T> {
    #[inline]
    fn empty() -> Self {
        Self::new()
    }
    #[inline]
    fn preset(value: T) -> Self {
        Self::from(value)
    }
    #[inline]
    fn get_or_init(&self, init: impl FnOnce() -> T) -> &T {
        std::sync::OnceLock::get_or_init(self, init)
    }
    #[inline]
    fn get(&self) -> Option<&T> {
        std::sync::OnceLock::get(self)
    }
}

impl<T> SingletonCell<T> for OnceCell<T> {
    #[inline]
    fn empty() -> Self {
        Self::new()
    }
    #[inline]
    fn preset(value: T) -> Self {
        Self::from(value)
    }
    #[inline]
    fn get_or_init(&self, init: impl FnOnce() -> T) -> &T {
        OnceCell::get_or_init(self, init)
    }
    #[inline]
    fn get(&self) -> Option<&T> {
        OnceCell::get(self)
    }
}

#[cfg(test)]
mod tests {
    use super::{SingleThread, SingletonCell, Storage};

    type Cell = <SingleThread as Storage>::Cell<u32>;

    #[test]
    fn empty_cell_runs_init_exactly_once() {
        let cell = Cell::empty();
        let mut calls = 0;
        assert_eq!(
            *cell.get_or_init(|| {
                calls += 1;
                7
            }),
            7
        );
        assert_eq!(*cell.get_or_init(|| 8), 7);
        assert_eq!(calls, 1);
    }

    #[test]
    fn preset_cell_never_runs_init() {
        let cell = Cell::preset(3);
        assert_eq!(cell.get(), Some(&3));
        assert_eq!(*cell.get_or_init(|| 9), 3);
    }

    #[cfg(feature = "std")]
    #[test]
    fn thread_safe_cell_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<<super::ThreadSafe as Storage>::Cell<u32>>();
    }
}
