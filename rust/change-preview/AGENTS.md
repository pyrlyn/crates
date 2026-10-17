# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The dry-run preview every command that changes the disk prints: a unified diff
or `--stat` lines for file edits, a `- path  size  N files` line for removals,
and a totals line. Shared so rtok and ketch print one format.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
