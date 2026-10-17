# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The one guard between text somebody else wrote and a terminal: it strips escape
sequences, control characters, bidi overrides and invisible characters. One
`Options` value says how much, so cox, ketch and rtok share one scanner and keep
their own output through a preset each. Pure, no dependencies.

## Rules

- A change to a preset changes a caller's output: add a test to `tests/callers.rs`
  that pins it, and say which caller it affects in the commit.
- The output of every preset must be idempotent and must not contain what it
  removes; `every_preset_is_idempotent_and_leaves_nothing_it_removes` checks it.
- `redact` (secrets) and `truncate` (display width) stay in cox-sanitize; they are not
  about untrusted control characters.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
