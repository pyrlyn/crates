# llm-wire-openai

OpenAI Chat Completions and Responses wires for LLM agent loops, also used for every OpenAI-compatible endpoint.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T1.6 | todo | P2 | 2 | 0% | |

### T1.6. Adopt llm-wire-openai in the coding agent (needs publication)

The coding agent's `agent-provider-openai` drops its `chat`, `responses` and `wire` modules and re-exports them from this crate, so `agent_provider::openai::chat::OpenAiChatProvider` and `agent_provider::openai::responses::OpenAiResponsesProvider` keep working. The `Transport` and `ProviderModel` arguments become `llm_http::Transport` and `llm_wire::ProviderModel` values built from the coding agent's own config types, and `agent_models::effort_for` becomes `llm_wire::effort_for`, which the Chat and Responses wires call. Blocked on publication of `llm-wire-openai`, `llm-http` and `llm-wire`: the coding agent can only depend on released crates, because a path dependency breaks the coding agent CI. Done when the coding agent builds against the published crates, its tests and snapshots pass unchanged, and the moved code and its fixtures are gone from `agent-provider-openai`.
