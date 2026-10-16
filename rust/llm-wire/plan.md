# llm-wire

The provider contract for LLM agent loops: the `Provider` trait, the neutral request and event types, and test-double helpers.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |

### T2. cox-protocol re-exports llm-wire (needs publication)

cox's `cox-protocol` drops its own copies of the moved items (`Provider`, `Request`, `ProviderEvent`, `Caps`, `ToolSpec`, `Usage`, `Risk`, `ProviderError`, `CallId`, `ArchiveId`, and the types they reach, see `done.md` T1) and re-exports them from this crate, so every path in cox keeps working. Blocked on publication: cox can only depend on a released `llm-wire`, because a path dependency breaks cox CI. Done when cox builds against the published crate, its `docs/protocol.jsonschema` test passes unchanged, and the moved code is gone from `cox-protocol`.
