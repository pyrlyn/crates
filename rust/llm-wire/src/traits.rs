//! The `Provider` trait: the one seam between an agent loop and a model
//! backend. The loop depends on this signature, never on a wire, so a real
//! provider and a test double are interchangeable.

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::errors::ProviderError;
use crate::types::{Caps, ProviderEvent, ProviderId, Request, Usage};

/// A model provider: turns a `Request` into a stream of `ProviderEvent`s.
/// Implemented by one type per backend and by test doubles; the agent loop
/// never depends on which.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Which provider this is.
    fn id(&self) -> ProviderId;
    /// What this provider implementation can do, so the loop can avoid
    /// sending it a request shape it does not support.
    fn capabilities(&self) -> Caps;
    /// Whether `model` on this wire takes `Content::Image` in a user
    /// message (T37.6). Per model because one Chat server hosts vision and
    /// text-only models side by side; `false` by default so a wire that
    /// never said so gets a notice instead of an image it would reject.
    fn accepts_images(&self, model: &str) -> bool {
        let _ = model;
        false
    }
    /// Streams a response, forwarding `ProviderEvent`s on `sink` as they
    /// arrive; returns the call's final `Usage` once the stream ends, or
    /// stops early if `cancel` fires.
    async fn stream(
        &self,
        req: Request,
        sink: mpsc::Sender<ProviderEvent>,
        cancel: CancellationToken,
    ) -> Result<Usage, ProviderError>;
    /// Counts tokens for a request without sending it, when the provider supports it.
    async fn count_tokens(&self, req: &Request) -> Result<u32, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check that `Provider` stays object-safe, since that is
    /// easy to break by accident (e.g. adding a generic method).
    #[test]
    fn provider_is_object_safe() {
        fn assert_object_safe<T: ?Sized>() {}
        assert_object_safe::<dyn Provider>();
    }
}
