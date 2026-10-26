# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Binary (and `cargo profile-check` subcommand): compare `[profile.*]` of Cargo manifests
with a template, Cargo inheritance resolved. The text output grammar in the README is a
contract for agents: change it only together with the README and `tests/cli.rs`.

## Commands

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```
