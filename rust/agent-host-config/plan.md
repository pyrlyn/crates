# agent-host-config

https://github.com/pyrlyn/crates

Register and unregister an MCP server in an agent host's JSON config file, changing only our entry.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |

### T2. Adopt agent-host-config in the token CLI (needs publication)

Replace `register_mcp`, `register_server`, `unregister_server`, `entry_at` and the atomic write in `tokcli-agent-sdk` with this crate. It stays `publish = false` here, so the token CLI can depend on it only after the creator publishes it (or accepts a git dependency). Done when the token CLI's host installers call this crate, its own copies are deleted, and the token CLI's tests pass. The token CLI keeps its ownership check (`unregister_owned`, `judge_owned`) and its `_backup/` pruning, which are not part of this crate.
