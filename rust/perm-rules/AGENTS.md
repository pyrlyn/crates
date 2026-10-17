# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The permission rule engine shared by cox and aulo: rule strings in Claude
Code's `Tool(subject)` grammar (`Bash(git commit:*)`, `Edit(src/**)`,
`mcp__srv__*`, `WebFetch(domain:x.com)`), the decision order (deny, mode, allow,
ask, session grant, policy, risk fallback) and session-grant bookkeeping. Pure:
no I/O, no host types, no serde.

## Rules

- Tool kinds are open strings. A host adds its own kinds (`App`, `Site`) with
  `Grammar::alias` and `Grammar::path_tool`; do not hard-code a host's tools.
- A malformed rule is a `RuleError`, never a skipped guard.
- No `unwrap`, `expect` or `panic` outside tests (clippy-denied).
- Published to crates.io from this repository.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
