# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The client half of an MCP host over `rmcp` 3.x: it finds the servers a project
declares (`.mcp.json`, `~/.claude.json`, the host's config), connects them all
without letting one failure stop the rest, and exposes their tools namespaced
(`mcp__<server>__<tool>`) and deferred. OAuth runs through an injectable token
store; a server's elicitation request is mapped onto questions a person
answers. The MCP *server* side (offering a host's own tools) is not here.

Shared so cox, and later aulo, drive MCP servers one way.

## Rules specific to this crate

- Depend on no application crate. A host maps its own config and tool types
  onto `ServerConfig`, `ToolSpec` and `ToolOutput`.
- Fail open: a server that cannot start, authenticate or list its tools is a
  notice string, never an error out of `connect_all`; a failed call is an
  error result the model can read, never a panic.
- `.mcp.json` and `~/.claude.json` are foreign files: read only their
  `mcpServers` entries, never write them, name the file in any parse error.
  `discovery` is the only module that reads them.
- A server's URLs, tool annotations and elicitation schemas are untrusted
  input. Keep the guards in `elicit` (http(s)-only URLs, consent before a
  browser opens) and `auth::open_browser` (no shell between the URL and the
  opener).
- Tests never touch the OS keychain or the process environment: inject the
  token store (`auth::Memory`) and the variable lookup (`discover_with`).

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
