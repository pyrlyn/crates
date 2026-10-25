# proc-sandbox

Confine a child process (Seatbelt, bubblewrap, Landlock plus seccomp) and the paths handed to it.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |

### T2. Adopt proc-sandbox in the coding agent (needs publication)

`agent-sandbox` in `apps/agent` still carries its own copy of this code. Once
`proc-sandbox` is published, replace `agent-sandbox` with a dependency on it: map
`agent_protocol::{SandboxMode, LinuxBackend, SandboxPolicy}` to the crate's
types, and map `path::ConfineError` to `ToolError::Confined` / `ToolError::Io`
at the one `confine` call site. Done when `agent-sandbox` is deleted and the coding agent's
sandbox and confine tests pass against this crate.
