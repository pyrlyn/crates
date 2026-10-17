# llm-wire-openai

OpenAI Chat Completions and Responses wires for LLM agent loops, also used for every OpenAI-compatible endpoint.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T1.6 | todo | P2 | 2 | 0% | |

### T1.6. Adopt llm-wire-openai in cox (needs publication)

cox's `cox-provider-openai` drops its `chat`, `responses` and `wire` modules and re-exports them from this crate, so `cox_provider::openai::chat::OpenAiChatProvider` and `cox_provider::openai::responses::OpenAiResponsesProvider` keep working. The `Transport` and `ProviderModel` arguments become `llm_http::Transport` and `llm_wire::ProviderModel` values built from cox's own config types, and `cox_models::effort_for` becomes `llm_wire::effort_for`, which the Chat and Responses wires call. Blocked on publication of `llm-wire-openai`, `llm-http` and `llm-wire`: cox can only depend on released crates, because a path dependency breaks cox CI. Done when cox builds against the published crates, its tests and snapshots pass unchanged, and the moved code and its fixtures are gone from `cox-provider-openai`.
