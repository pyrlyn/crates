//! HTTP plumbing every network provider wire needs on top of a plain
//! `reqwest` call: connection-pooled client construction, credential
//! resolution (env var, then platform keyring), auth headers and non-2xx →
//! [`ProviderError`](llm_wire::ProviderError) mapping ([`http`]); generic
//! Server-Sent-Events framing (`sse`); and the retry/backoff policy wrapped
//! around one provider stream ([`retry`]), plus the [`Transport`] section
//! every wire's constructor takes.
//!
//! Extracted from cox's `cox-provider-http` so cox and aulo share one copy.
//! The error and event types come from [`llm_wire`].
//!
//! # Features
//!
//! `keyring`, `reqwest` and `sse` are on by default; `test-util` is not. A host
//! that keeps secrets in its own store and sends requests through its own HTTP
//! stack turns the three off
//! (`default-features = false`) and builds without `keyring`, `reqwest` or
//! `eventsource-stream`.
//!
//! | Feature | Gates | Pulls in |
//! | --- | --- | --- |
//! | `keyring` | `http::resolve_key` (env var, then the platform keyring) | `keyring` |
//! | `reqwest` | `http::client_with_timeout` | `reqwest` |
//! | `sse` | the `sse` module (`sse_stream`, `parse_sse_str`) | `eventsource-stream`, `bytes`, `futures` |
//! | `test-util` | the `cassette` module (record and replay HTTP bodies, secrets masked); off by default | `serde`, `llm-wire/test-util` |
//!
//! Always available: `http::resolve_key_with` (the lookup injected),
//! `keyring_enabled`, `bearer`, `api_key`, `error_message`,
//! `parse_context_too_long` and `map_http_error` (on `http` crate types,
//! the same ones reqwest re-exports), the whole `retry` module, and
//! [`Transport`].
//!
//! On wasm32-unknown-unknown, `retry::stream_with_retry` is absent because
//! tokio's timer cannot run there; `retry::stream_with_retry_with` takes the
//! backoff sleep from the caller instead.

#[cfg(feature = "test-util")]
pub mod cassette;
pub mod http;
pub mod retry;
#[cfg(feature = "sse")]
pub mod sse;
mod transport;

pub use transport::Transport;
