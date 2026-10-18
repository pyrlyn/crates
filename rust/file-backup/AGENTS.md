# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Copy of a file to `<name>.bak-<unix-seconds>`, beside it or in a pruned subfolder. Shared so
ketch and rtok do not each invent a backup naming scheme.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
