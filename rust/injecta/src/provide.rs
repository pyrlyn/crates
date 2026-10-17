//! The resolution contract: `Provide<T>` (a container can hand out a `T`),
//! `Injectable<C>` (a type can build itself from container `C`), `Override<T>`
//! (a cached entry can be replaced) and the `Resolve` extension that callers
//! use. Separate from the macros so hand-written containers and generic code
//! can name the same bounds.

/// Deepest dependency chain a container accepts. `container!` checks every
/// entry against it at compile time; a cycle makes the depth infinite, which
/// rustc reports as a const-evaluation cycle before this limit is reached.
pub const MAX_DEPTH: usize = 64;

/// A container that can produce a `T`.
///
/// `container!` implements this once per declared entry. In generic code,
/// write the bound `C: Provide<T>` and call [`Resolve::resolve`].
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no provider for `{T}`",
    label = "`{T}` is not registered in `{Self}`",
    note = "declare it in `injecta::container!` as `instance {T}`, `singleton {T}` or `transient {T}`",
    note = "inside a `scope`, a root `singleton` cannot depend on a `scoped` value: make the dependent `scoped` or `transient`"
)]
pub trait Provide<T> {
    /// Length of the longest dependency chain below `T` (0 for leaves and
    /// factories). Used only for the compile-time cycle and depth check.
    const DEPTH: usize;

    /// Produces the value. Prefer [`Resolve::resolve`] at call sites.
    fn provide(&self) -> T;
}

/// A container that holds a `T` and can lend it out without cloning: the
/// `instance`, `singleton` and `scoped` entries.
///
/// `container!` implements this for every cached entry. Call it through
/// [`Resolve::get`]; it skips the `Clone` (for `Arc<T>`, the reference-count
/// increment and decrement) that [`Resolve::resolve`] pays.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not hold a `{T}` it can lend",
    label = "`{T}` is not an `instance`, `singleton` or `scoped` entry of `{Self}`",
    note = "`get` borrows cached values only; `transient` values are built on every call: use `resolve::<{T}>()`",
    note = "if `{T}` is not registered at all, declare it in `injecta::container!`"
)]
pub trait ProvideRef<T>: Provide<T> {
    /// Borrows the value, creating a lazy `singleton`/`scoped` value first if
    /// needed. Prefer [`Resolve::get`] at call sites.
    fn provide_ref(&self) -> &T;
}

/// A type that knows how to build itself from any container `C` that provides
/// its dependencies.
///
/// Implement it with `#[derive(Injectable)]` (struct fields are the
/// dependencies) or `#[injectable]` on an `impl` block with an `#[inject]`
/// constructor. Hand-written impls are allowed but rarely needed.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be built by a container",
    label = "`{Self}` is not `Injectable`",
    note = "add `#[derive(injecta::Injectable)]` to `{Self}`, or put `#[injecta::injectable]` on an `impl` block with one `#[inject]` constructor",
    note = "for a foreign type or a trait object, register a factory instead: `singleton Arc<dyn Trait> = |c| Arc::new(c.build::<Impl>())`"
)]
pub trait Injectable<C: ?Sized>: Sized {
    /// Longest dependency chain below `Self`, plus one.
    const DEPTH: usize;

    /// Builds `Self`, resolving every dependency from `container`.
    fn inject(container: &C) -> Self;
}

/// A container entry whose cached value can be replaced, typically by a test
/// double: `App::new(config).with(fake_db)`.
#[diagnostic::on_unimplemented(
    message = "`{T}` cannot be overridden in `{Self}`",
    label = "no `singleton` or `scoped` entry of type `{T}`",
    note = "only `singleton` (root) and `scoped` (scope) entries are cached and can be overridden; `instance` values are constructor arguments",
    note = "`transient` values are rebuilt on every resolve: override their dependencies instead"
)]
pub trait Override<T> {
    /// Replaces the cached value. Later resolves return clones of `value`.
    fn set(&mut self, value: T);
}

/// The call-site API, implemented for every type.
///
/// `resolve` returns a registered entry by value; `get` borrows a cached one;
/// `build` constructs an unregistered `Injectable` type from the registered
/// entries (handy inside factories).
pub trait Resolve {
    /// Returns the container's `T`: a clone of an instance or singleton, or a
    /// freshly built transient.
    #[inline]
    fn resolve<T>(&self) -> T
    where
        Self: Provide<T>,
    {
        self.provide()
    }

    /// Borrows a cached entry (`instance`, `singleton` or `scoped`) without
    /// cloning it: the cheapest way to use a shared value at a call site,
    /// and the one that scales across threads (no reference-count traffic).
    #[inline]
    fn get<T>(&self) -> &T
    where
        Self: ProvideRef<T>,
    {
        self.provide_ref()
    }

    /// Builds an `Injectable` type that is not itself registered, resolving
    /// its dependencies from this container.
    #[inline]
    fn build<T>(&self) -> T
    where
        T: Injectable<Self>,
    {
        T::inject(self)
    }
}

impl<C: ?Sized> Resolve for C {}

#[cfg(test)]
mod tests {
    use super::{Injectable, Provide, ProvideRef, Resolve};

    struct Hand {
        answer: u32,
    }

    impl Provide<u32> for Hand {
        const DEPTH: usize = 0;
        fn provide(&self) -> u32 {
            self.answer
        }
    }

    impl ProvideRef<u32> for Hand {
        fn provide_ref(&self) -> &u32 {
            &self.answer
        }
    }

    #[test]
    fn get_borrows_without_cloning() {
        let hand = Hand { answer: 21 };
        assert!(core::ptr::eq(hand.get::<u32>(), &raw const hand.answer));
    }

    struct Doubled(u32);

    impl<C: Provide<u32> + ?Sized> Injectable<C> for Doubled {
        const DEPTH: usize = 1 + <C as Provide<u32>>::DEPTH;
        fn inject(container: &C) -> Self {
            Self(container.resolve::<u32>() * 2)
        }
    }

    #[test]
    fn resolve_returns_hand_written_provider_value() {
        let hand = Hand { answer: 21 };
        assert_eq!(hand.resolve::<u32>(), 21);
    }

    #[test]
    fn build_constructs_unregistered_injectable() {
        let hand = Hand { answer: 21 };
        assert_eq!(hand.build::<Doubled>().0, 42);
        assert_eq!(<Doubled as Injectable<Hand>>::DEPTH, 1);
    }
}
