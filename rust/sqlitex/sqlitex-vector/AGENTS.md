# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

An HNSW vector index persisted in one SQLite file, with optional `i8`
quantization. Extracted from cox's `cox-vecdb` (branch `exp/vector-bench`),
with the cox-only engines removed.

## Rules

- `unsafe` lives only in `src/sqlite.rs` (FFI) and `src/simd.rs` (SIMD loads,
  feature `simd`); every block keeps a SAFETY note.
- `bench-qdrant/` stays outside the workspace and unpublished, so qdrant's
  tree never enters the root `Cargo.lock`. Never commit a local `[patch]` path.
- Benchmarks are manual: do not add them to CI or `cargo test`.
- Changing the page or meta layout breaks existing files: bump the minor
  version and say so in the changelog.

## Commands

```bash
cargo test -p sqlitex-vector
cargo clippy -p sqlitex-vector --all-targets --all-features -- -D warnings
cargo clippy --manifest-path bench-qdrant/Cargo.toml --all-targets -- -D warnings
```
