# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Paths a git working tree changed relative to a base ref: commits since the merge
base, staged, unstaged, untracked-not-ignored, deleted files and both sides of a
rename. Shells out to the `git` CLI (no shell, `-z` output) instead of `gix`.
Paths are relative to the repository top level.

## Rules

- Fail safe: when the answer cannot be computed exactly, return an `Error`; callers
  never treat it as "nothing changed": fail loudly or run everything.
- Tests build throwaway repos with `tempfile`; keep host git config out of them.

## Commands

```bash
cargo test -p git-changed-paths
cargo clippy -p git-changed-paths --all-targets
cargo fmt
```
