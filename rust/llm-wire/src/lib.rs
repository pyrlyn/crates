//! The provider contract an agent loop depends on instead of any model
//! backend: the [`Provider`](traits::Provider) trait, the neutral
//! [`Request`](types::Request) it takes and the
//! [`ProviderEvent`](types::ProviderEvent) stream it returns, with the types
//! those carry (`Caps`, `ToolSpec`, `Usage`, `Risk`, `ProviderError`, ...).
//!
//! Shapes and serde/schemars derives only; no wire format, no I/O. A wire
//! crate implements `Provider` and translates `Request` to its own format.
//!
//! - [`ids`] — `CallId`, `ArchiveId`: ULID newtypes.
//! - [`errors`] — `ProviderError`.
//! - [`model`] — `ProviderModel` (a configured model entry) and `Capabilities` (what it supports).
//! - [`effort`] — the one rule for which effort each wire sends (`effort_for`), and `supports_adaptive_thinking`.
//! - [`types`] — `Request`, `ProviderEvent`, `Caps`, `ToolSpec`, `Usage`, `Risk` and everything reachable from them.
//! - [`traits`] — `Provider`.
//! - `test_util` (feature `test-util`) — scenario and cassette helpers for test doubles.

pub mod effort;
pub mod errors;
pub mod ids;
pub mod model;
pub mod traits;
pub mod types;

#[cfg(feature = "test-util")]
pub mod test_util;

pub use effort::{Api, WireEffort, effort_for, supports_adaptive_thinking};
pub use errors::ProviderError;
pub use ids::{ArchiveId, CallId};
pub use model::{Capabilities, ProviderModel};
pub use traits::Provider;
pub use types::{
    ArchiveRef, Caps, Concurrency, Content, Effort, Job, Message, ModelId, ProviderEvent,
    ProviderId, Request, Risk, Role, StopReason, SystemBlock, Thinking, Tier, ToolSpec, Usage,
};
