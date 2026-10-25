# agent-ext

Agent extension points: SKILL.md discovery with a deferred skill tool, and Claude-Code-style shell hooks that fail open.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |

### T2. Adopt agent-ext in the coding agent (needs publication)

Replace `skills.rs`, `hooks.rs` and `frontmatter.rs` in the coding agent's `crates/agent-ext` with this crate and delete the copies. The coding agent wraps `SkillTool` in its own `Tool` impl (risk `ReadOnly`, `Parallel`, `ToolError::NotFound` for `SkillNotFound`) and maps this crate's `HookEvent`, `HookOutcome`, `HookConfig` and `HooksConfig` to its `agent_protocol` types (or re-exports them from there). `presence::with_context` becomes the crate's `hooks::with_context`. Blocked until the crate is published: it is `publish = false` today, so the crate must first be published (or the release flow set up for it).
