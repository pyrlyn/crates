# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Register and unregister an MCP server in an agent host's JSON config file (Claude
Code `~/.claude.json`, Cursor `~/.cursor/mcp.json`, and hosts with another server
map such as OpenCode's `mcp`), changing only our entry. Extracted from
`rtok-agent-sdk` so rtok and aulo share one implementation. Published to
crates.io from this repository.

## Rules for changes

- The host owns its file. Never rewrite it from a parsed value: edit the syntax
  tree (`jsonc-parser` `cst`), so every byte outside our entry survives. The
  round-trip tests in `src/lib.rs` must keep passing byte-for-byte.
- A file that is not JSON, or a root or server map that is not an object, is an
  error naming the file. Never replace the user's value.
- Writes go through `write` (backup through `file-backup`, then an atomic swap).
- Callers pass the server name and command. Nothing here names a product.
- Tests use temp directories only, never the real home directory.

## Commands

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
