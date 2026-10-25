# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The one entry point for file and directory work in the package manager, the token CLI and the other projects:
path identity (`\\?\`-free canonical paths, Windows case folding, a lexical join), atomic
file replacement, reading where a missing file is empty, tree usage, copy and removal,
and sibling `.bak-<unix-seconds>` backups. Std plus `dunce` only by default, Rust 1.86, so
The package manager can depend on it; the opt-in `clone` feature adds `reflink-copy` for `tree::clone_tree`.

A new file or directory helper in any project belongs here, not in the project. Keep each
function's guarantees in its doc comment (what it follows, what it never touches, what a
missing path means), since callers rely on them at trust boundaries.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```
