//! Crate root: declares the modules and the public surface. The user guide is
//! `GUIDE.md`, included here so every example in it runs as a doctest.
#![doc = include_str!("../GUIDE.md")]
#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod hooks;
mod info;
mod provide;
mod storage;

#[cfg(feature = "tracing")]
pub use hooks::TracingHooks;
pub use hooks::{Hooks, NoHooks};
pub use info::{Container, Description, Lifetime, ProviderInfo};
pub use provide::{Injectable, MAX_DEPTH, Override, Provide, ProvideRef, Resolve};
#[cfg(feature = "std")]
pub use storage::ThreadSafe;
pub use storage::{DefaultStorage, SingleThread, SingletonCell, Storage};

#[cfg(feature = "macros")]
pub use injecta_macros::{Injectable, container, injectable};

/// Runs the README examples as doctests so the README cannot drift.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;

/// Items the generated code names. Not part of the public API: paths and
/// signatures may change in any release.
#[doc(hidden)]
pub mod __private {
    pub use alloc::boxed::Box;
    pub use alloc::sync::Arc;
    pub use core::clone::Clone;
    pub use core::default::Default;

    /// Calls a `container!` factory closure. Passing the closure as an
    /// argument with a known `FnOnce(&C) -> T` bound is what lets `|c| ...`
    /// infer `c: &C`; an immediately invoked closure would not.
    #[inline]
    pub fn call_factory<C: ?Sized, T>(container: &C, factory: impl FnOnce(&C) -> T) -> T {
        factory(container)
    }

    /// The depth check `container!` emits; a function rather than an inline
    /// `<=` so a constant `0` does not trip lints in the caller's crate.
    #[must_use]
    pub const fn within_max_depth(depth: usize) -> bool {
        depth <= crate::MAX_DEPTH
    }

    /// `max` usable in the `DEPTH` constants the macros generate.
    #[must_use]
    pub const fn depth_max(a: usize, b: usize) -> usize {
        if a > b { a } else { b }
    }
}
