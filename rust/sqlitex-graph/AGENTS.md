# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

A code-symbol store in SQLite (normalized `roots`/`files`/`names`/`defs`/`refs`,
per-root FTS5) with the reference/call graph kept in immutable CSR segments and
queried through table-valued functions (`graph_impact`, `graph_neighbors`,
`graph_callers`, `graph_callees`). One crate, two shapes: an rlib (`Store`,
`register`) and a SQLite loadable extension (cdylib, `loadable` feature).
Extracted from the rtok graph experiment (`~/Grok/rtok-graph-ext`); rtok is not
switched over. How it works: `docs/design.md`.

## Rules

- SQLite tables are the source of truth. The CSR segments are a cache: anything
  in them must be rebuildable from `defs`/`refs` (`store::build_base`), and the
  tests compare against that rebuild.
- Readers never take a lock. Writers go through `Store` (one at a time); merges
  run off the write path and swap a pointer.
- `unsafe` only in `src/ffi.rs` (the libsqlite3-sys wrapper, vtab callbacks, entry
  points), each block with a `SAFETY` note; every SQLite callback in `catch_unwind`.
- No rusqlite: depend on `libsqlite3-sys` directly, at the workspace's (diesel's) version.
- rtok-specific code (its schema, its paths) stays behind the `rtok-import`
  feature: `src/rtok.rs`, `examples/import_rtok.rs`, `examples/qps.rs`,
  `tests/equivalence.rs`, `scripts/`.

## Commands

```bash
cargo test -p sqlitex-graph
cargo clippy -p sqlitex-graph --all-targets -- -D warnings
cargo clippy -p sqlitex-graph --no-default-features --features loadable --all-targets -- -D warnings
cargo clippy -p sqlitex-graph --features rtok-import --all-targets -- -D warnings
RTOK_OLD_DB=/path/to/rtok.db cargo test -p sqlitex-graph --release --features rtok-import --test equivalence -- --ignored
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
