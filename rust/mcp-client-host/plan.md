# mcp-client-host

https://github.com/pyrlyn/crates

MCP client host over rmcp 3.x: `.mcp.json` discovery, fail-open `connect_all`, OAuth with an injectable token store, elicitation mapping and deferred tools.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 3 | 0% | |

### T2. Adopt mcp-client-host in cox (needs publication)

`cox-mcp`'s client parts (`client`, `discovery`, `auth`, `elicit`) were copied here (T1) but cox still carries its own copy. Replace them with this crate: map `cox_protocol::config::McpServerConfig` onto `ServerConfig`, wrap `McpTool` in cox's `Tool` trait (`spec`, `subject`, `call` with `ToolCx::cancel`), build `Host` from cox's secrets, prompt, asker and `CHILD_ENV_ALLOWLIST`, and use `Keyring::new("cox").with_off_switch("COX_KEYRING")` behind the `keyring` feature. `cox-mcp` keeps only the server side. Done when cox's tests pass on the crate and its copies are deleted. Blocked until the crate is published (cox depends on registry crates, not paths into this repository).
