# agent-loop

The neutral turn loop of an LLM agent: submissions in, events out, with tool dispatch, approvals and interrupt behind traits.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P1 | 3 | 0% | |

### T2. cox-core uses agent-loop (T1.16, needs publication)

cox's `cox-core` drives its turns through this crate instead of its own loop (`session.rs` `step`/`finish`, `turn.rs` `consume_provider`/`run_signed_tools`/`gate`/`ask`): cox implements `Tools`, `Approvals`, `Context` and `EventSink` (see `AGENTS.md`, Design) and maps `Event` onto its own enum, then deletes the duplicated loop code. Decide with the creator whether cox takes the deliberate differences listed in `AGENTS.md` or the crate gets switches for cox's old behaviour. Blocked on publication: cox can only depend on released crates (`llm-wire` and `agent-loop`), because a path dependency breaks cox CI. Done when cox CI is green on the published crate, its golden event snapshots pass, and the moved code is gone from `cox-core`.
