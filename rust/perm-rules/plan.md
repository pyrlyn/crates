# perm-rules

Permission rule engine for tool calls: `Tool(subject)` grammar, deny/allow/ask decision order and a risk fallback, extracted from agent-permission.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 3 | 0% | |

### T2. Adopt perm-rules in the coding agent (needs publication)

agent-permission becomes a thin adapter over this crate: map `ToolCall`, `PermissionsConfig`, `CoreError::Config` and the coding agent's `Why`/`DecidedBy` (which keep `SandboxDenied`, `User` and `Hook`) onto `Call`, `RuleSet`, `RuleError`; keep the coding agent's default deny list and the bash-parser tests there. Blocked until `perm-rules` is published (flip `publish = false`) because the coding agent depends on crates.io versions. Done when the coding agent's `permission` and `policy_matrix` tests pass against the crate.
