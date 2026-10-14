# agent-host-config

Register and unregister an MCP server in an agent host's JSON config file, changing only our entry.

```rust
use agent_host_config::{Apply, NO_CHANGES, register_mcp, unregister_mcp};

let apply = Apply { dry_run: false, backup: true };
let path = std::path::Path::new("/home/me/.cursor/mcp.json");
// "mcpServers.demo: demo-bin mcp", or NO_CHANGES on a second run
let report = register_mcp(&apply, path, "demo", "demo-bin", &["mcp"])?;
unregister_mcp(&apply, path, "demo")?; // "- mcpServers.demo"
```

- **Only our entry.** The file is edited through a lossless syntax tree (`jsonc-parser`), so every other key keeps its value, position, spelling, comments and indentation. The object that gains our entry is made multi-line and its previous last entry gains a comma; nothing else moves. Unregistering takes our entry, and the server map if it ends up empty.
- **Atomic.** A sibling temp file is renamed over the config. A symlinked config keeps its link; permissions (a 0600 `~/.claude.json`) survive.
- **Reversible.** `Apply::backup` copies the old file to `<name>.bak-<unix-seconds>` first, through [`file-backup`](../file-backup).
- **Idempotent and dry-runnable.** A second apply returns `NO_CHANGES` and writes nothing; `Apply::dry_run` returns the same report and writes nothing.
- **Never clobbers.** A file that is not JSON (comments and trailing commas are accepted), or a root or server map that is not an object, is an error naming the file and position.
- **Other shapes.** `register_server` / `unregister_server` take a dotted key (`mcp.servers`) and any entry value; `entry_at` reads an entry so a caller can check ownership before removing it by name.

Hosts that keep MCP servers in TOML (Codex) are not covered yet; see `ideas.md`.
