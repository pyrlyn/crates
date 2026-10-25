# llm-wire

The provider contract for LLM agent loops: the `Provider` trait, the neutral request and event types, and test-double helpers.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |

### T2. agent-protocol re-exports llm-wire (needs publication)

The coding agent's `agent-protocol` drops its own copies of the moved items (`Provider`, `Request`, `ProviderEvent`, `Caps`, `ToolSpec`, `Usage`, `Risk`, `ProviderError`, `CallId`, `ArchiveId`, and the types they reach, see `done.md` T1) and re-exports them from this crate, so every path in the coding agent keeps working. Blocked on publication: the coding agent can only depend on a released `llm-wire`, because a path dependency breaks the coding agent CI. Done when the coding agent builds against the published crate, its `docs/protocol.jsonschema` test passes unchanged, and the moved code is gone from `agent-protocol`.
