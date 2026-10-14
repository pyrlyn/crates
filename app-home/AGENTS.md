# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The one place every project resolves the user home, its own `$APP_HOME` or
`~/.app` directory, the XDG base directories and a leading `~`. Shared so rtok,
cox, aulo, ketch, runa and swarfr agree on empty variables, the Windows
`USERPROFILE` fallback and relative XDG values.

It stays std only, edition 2021 and Rust 1.86: ketch is the oldest consumer.
Every resolver takes an injectable variable lookup so tests never touch the
process environment.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
