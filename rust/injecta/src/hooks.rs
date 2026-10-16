//! Lifecycle hooks: the one place a container reports what it does. The
//! default `NoHooks` compiles to nothing, so observability costs nothing
//! unless a container opts in with `#[injecta(hooks = ...)]`.

use crate::info::ProviderInfo;

/// Observer of a container's activity. Every method has an empty default.
///
/// Hooks are created with `Default` when the container is constructed and
/// shared by its scopes. Keep them cheap: `on_resolve` runs on every resolve.
pub trait Hooks: Default {
    /// A `singleton` or `scoped` value was just created (once per cell).
    #[inline]
    fn on_create(&self, _info: &ProviderInfo) {}

    /// An entry is being resolved (every call, cached or not).
    #[inline]
    fn on_resolve(&self, _info: &ProviderInfo) {}
}

/// The default hooks: do nothing, cost nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoHooks;

impl Hooks for NoHooks {}

/// Emits `tracing` events: `debug` on creation, `trace` on resolve.
#[cfg(feature = "tracing")]
#[derive(Debug, Default, Clone, Copy)]
pub struct TracingHooks;

#[cfg(feature = "tracing")]
impl Hooks for TracingHooks {
    fn on_create(&self, info: &ProviderInfo) {
        tracing::debug!(provider = info.type_name, lifetime = ?info.lifetime, "injecta: created");
    }

    fn on_resolve(&self, info: &ProviderInfo) {
        tracing::trace!(provider = info.type_name, lifetime = ?info.lifetime, "injecta: resolve");
    }
}
