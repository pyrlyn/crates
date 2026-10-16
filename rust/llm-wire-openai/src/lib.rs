//! The OpenAI-shaped providers. `responses` covers the Responses API
//! (`POST /v1/responses`), which is what OpenAI's own models use; `chat`
//! covers the Chat Completions subset that every OpenAI-compatible server
//! speaks (Ollama, vLLM, LM Studio, llama.cpp, OpenRouter, DeepSeek, Gemini's
//! compatibility endpoint, xAI).
//!
//! Both are separate from each other rather than generalised behind one
//! translator: the wire formats disagree about where system text, reasoning
//! and tool results live, and a shared abstraction would have to be
//! re-specialised at every one of those points.
//!
//! Each wire is a pure request translator (`build_body`), a pure SSE to
//! [`ProviderEvent`](llm_wire::ProviderEvent) state machine, and a thin
//! client implementing [`Provider`](llm_wire::Provider) over [`llm_http`].
//! Everything a server sends is untrusted input: the state machines never
//! panic on a malformed frame and the HTTP layer maps every failure to a
//! `ProviderError`.
//!
//! Extracted from cox's `cox-provider-openai` so cox and aulo share one copy.
//! The section and model types the wires read (`llm_http::Transport`,
//! `llm_wire::ProviderModel`, `llm_wire::Capabilities`) live in the crates
//! below, so a host holds one copy of each.

// Imported at the crate root so the wires' `crate::http`, `crate::retry` and
// `crate::sse` paths resolve exactly as they did inside cox.
use llm_http::{http, retry, sse};

pub mod chat;
pub mod responses;
mod wire;
