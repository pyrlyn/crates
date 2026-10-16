# llm-http

HTTP plumbing for LLM provider wires: credential lookup, auth headers, error mapping, retry with backoff and SSE framing.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |

### T2. Adopt llm-http in cox (needs publication)

cox's `cox-provider-http` drops its own `http`, `retry` and `sse` modules and re-exports them from this crate, so `cox_provider::http::resolve_key_with` and every other old path keep working. `resolve_key` gains the `service` and `switch_env` arguments cox passes as `"cox"` and `COX_KEYRING`, and `cox_protocol::config::keyring_enabled` delegates to `llm_http::http::keyring_enabled`. Blocked on publication of `llm-http` and `llm-wire`: cox can only depend on released crates, because a path dependency breaks cox CI. Done when cox builds against the published crates, its tests pass unchanged, and the moved code is gone from `cox-provider-http`.
