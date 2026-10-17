# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The provider contract an agent loop depends on instead of any model backend:
the `Provider` trait, the neutral `Request` it takes, the `ProviderEvent`
stream it returns, and the types those carry. Extracted from cox's
`cox-protocol` so cox and aulo share one contract. Shapes and serde/schemars
derives only: no wire format, no I/O.

## Rules

- **Behaviour-neutral with cox.** Serde tags, field names and the schemars doc
  strings are what cox's `docs/protocol.jsonschema` and its rollout files are
  made of. Change one only together with cox, never alone.
- **No `unwrap`, `expect` or `panic!` outside tests** (clippy denies them).
- **Doc comments on every public item** (`missing_docs` is on).
- **`test-util` stays optional.** The scenario parser and cassette helpers sit
  behind the feature so a production build carries no TOML and no hashing.
- Published to crates.io from this repository.

## Commands

```bash
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
