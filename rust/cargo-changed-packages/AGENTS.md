# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Library: given repo-relative changed paths, the workspace packages they affect,
reverse dependencies included, via `guppy` and `determinator`. Anything that
changes the package graph or the build selects everything (`all: true`); it
never answers "nothing" when it cannot tell. The `cargo metadata` cache (`src/cache.rs`)
must treat every doubt as a miss, never as a hit.

## Commands

```bash
cargo test   # tests/cache.rs counts cargo runs with a shim
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
