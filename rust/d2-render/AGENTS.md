# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Renders D2 diagrams. The default backend (feature `native`, on by default) is
`src/native/`, a pure-Rust rewrite of a D2 subset; every file there keeps its
MPL-2.0 header. Feature `cli` adds `src/cli.rs`, which shells out to the `d2`
executable and parses its stderr. Feature `png` rasterizes native SVG with
resvg. README.md documents the public API, the native-vs-CLI decision and the
native supported/unsupported table; keep them in sync with the code.

## Rules

- The default build must never need the `d2` binary: anything that runs d2
  lives behind `#[cfg(feature = "cli")]`, and so do tests that call it.
- Native features D2 has but the port does not draw must produce a warning
  (or a diagnostic when ignoring them would change the meaning), never be
  dropped silently. Update the README table when coverage changes.
- Native diagnostics should match d2's message, line and column; add a
  fixture to `diagnostics_match_cli` in `tests/native.rs` when adding one.
- Only pass d2 flags that `d2 --help` lists; check new ones against the
  installed d2 before adding them.
- Tests that need a real d2 must soft-skip (`common::real_d2`) when it is missing.
- New dependencies must keep the lockfile buildable on the MSRV (1.86):
  `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p <dep>`.

## Commands

```bash
cargo test -p d2-render                       # default features, no d2 needed
cargo test -p d2-render --all-features
cargo clippy -p d2-render --all-targets -- -D warnings
cargo clippy -p d2-render --all-targets --no-default-features --features cli -- -D warnings
cargo clippy -p d2-render --all-targets --all-features -- -D warnings
cargo fmt
```
