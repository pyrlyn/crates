# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Drift tests for generated bindings: `Drift` compares generated text with a
committed file (unified diff on mismatch, rewrite under a bless switch), and
`cbindgen_header` / `csbindgen_file` render bindings to compare. Extracted from
scull's `crates/scull-ffi/tests/bindings.rs` and ketch's `ketch-capi` drift
test; scull, ketch and Mailune (B6) are its consumers.

## Rules

- A failing check is an `Error`, never a panic: the caller's test decides.
  `Debug` prints the same text as `Display` so `?` and `unwrap` in a test show
  the diff.
- `tests/fixtures/ffi` is read as text by the generators and never compiled.
  After changing it, regenerate with `ABI_DRIFT_BLESS=1 cargo test -p abi-drift`.
- `rust-version` stays at ketch's 1.86 while ketch is a consumer.
- Dependency versions follow scull's and ketch's locks. Bump them only with the creator.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
