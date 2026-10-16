# perm-rules

Permission rule engine for tool calls: `Tool(subject)` grammar, deny/allow/ask decision order and a risk fallback, extracted from cox-permission.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 3 | 0% | |

### T2. Adopt perm-rules in cox (needs publication)

cox-permission becomes a thin adapter over this crate: map `ToolCall`, `PermissionsConfig`, `CoreError::Config` and cox's `Why`/`DecidedBy` (which keep `SandboxDenied`, `User` and `Hook`) onto `Call`, `RuleSet`, `RuleError`; keep cox's default deny list and the bash-parser tests there. Blocked until `perm-rules` is published (flip `publish = false`) because cox depends on crates.io versions. Done when cox's `permission` and `policy_matrix` tests pass against the crate.
