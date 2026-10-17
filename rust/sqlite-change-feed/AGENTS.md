# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

A pull-based change feed for a SQLite database shared between processes:
`PRAGMA data_version` through Diesel, as a per-connection `ChangeToken`.
Extracted from cox's `cox-store` (`src/watch.rs`); cox and Mailune (S7) are its
consumers. It owns no connection and no thread; the application passes its
connection in.

## Rules

- The pragma is the only raw SQL here, because Diesel cannot model a PRAGMA.
  Keep any other statement out of this crate.
- Do not enable `libsqlite3-sys` features in `[dependencies]`: the
  application chooses bundled SQLite or SQLCipher. Tests bundle SQLite as a
  dev-dependency only.
- The cross-process test re-executes the test binary (`writer_process`);
  keep it, it is the one test that proves the feed's purpose.
- Dependency versions follow cox's and Mailune's locks. Bump them only with the creator.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
