# llm-wire-anthropic

The Anthropic Messages wire for LLM agent loops: request translation, SSE stream parser and a streaming client.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 3 | 0% | |

### T2. Adopt llm-wire-anthropic in the coding agent (T1.6, needs publication)

The coding agent's `agent-provider-anthropic` drops its own `request`, `stream`, `wire`, `build.rs` and `schema/` and re-exports them from this crate, so `agent_provider::anthropic::request::build_body` and every other old path keep working. The coding agent keeps what is agent-only: it builds `llm_http::Transport` from `agent_protocol::config::Transport` and passes `"the coding agent"` and `AGENT_KEYRING` to `AnthropicProvider::new`; the `agent-models` `Api::Anthropic` arm of `effort_for` and `supports_adaptive_thinking` become the one copy in `llm-wire` (`effort_for`, `supports_adaptive_thinking`). The coding agent's `Replay` keeps feeding cassettes through `AnthropicStream`. Blocked on publication of `llm-wire-anthropic`, `llm-http` and `llm-wire`: the coding agent can only depend on released crates, because a path dependency breaks the coding agent CI. Done when the coding agent builds against the published crates, its tests and snapshots pass unchanged, and the moved code is gone from `agent-provider-anthropic`.
