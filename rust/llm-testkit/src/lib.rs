//! Shared HTTP cassettes for tests of an LLM host.
//!
//! Mailune and other hosts that send requests through their own stack depend on
//! this crate instead of naming `llm-http`'s `test-util` feature themselves.
//! [`cassette`] is [`llm_http::cassette`]: record a request and response, mask
//! secrets, and replay the stored answer. Nothing in this crate implements a
//! second masker or opens a socket.

/// Record and replay HTTP bodies with secrets masked.
///
/// This is [`llm_http::cassette`](llm_http::cassette), re-exported.
#[doc(inline)]
pub use llm_http::cassette;
