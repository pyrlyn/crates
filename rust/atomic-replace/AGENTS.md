# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The one atomic file write every project uses: a sibling temp file, the old
file's permissions, symlinks kept, optional fsync of the file and its
directory, then a rename over the target. Shared so rtok, ketch, cox, swarfr,
aulo and runa stop carrying their own copies.

It stays std only, edition 2021 and Rust 1.86: ketch is the oldest consumer.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
