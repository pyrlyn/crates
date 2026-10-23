# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The name Mailune depends on for HTTP cassettes. It re-exports
`llm_http::cassette` from published `llm-http` 0.1.0 (feature `test-util`).
Recording, masking and replay stay in `llm-http`. This crate does not copy them
and does not open a socket.

## Rules

- Do not vendor or reimplement the cassette. If `llm_http::cassette` is not
  public, stop; do not fork the algorithm.
- Do not change `llm-http`'s public API from this crate. It is already published.
- Depend on `llm-http` 0.1.0 with `test-util` and without the default
  features (`reqwest`, `keyring`, `sse`). Use `path` and `version` together.
  A version-only crates.io dependency sits beside this workspace's path
  package and makes `cargo test -p llm-http` ambiguous. Do not copy the module.
- Tests call the re-export, never `llm_http` directly, and they do not touch
  the network.
- No `unwrap`, `expect` or `panic!` outside tests (clippy denies them), and no
  `unsafe` (forbidden).

## Commands

```bash
cargo test -p llm-testkit
cargo clippy -p llm-testkit --all-targets -- -D warnings
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
