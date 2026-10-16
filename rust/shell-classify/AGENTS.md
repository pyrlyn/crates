# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The bash risk classifier that cox's permission engine and aulo share:
`classify(line)` rates a command line `ReadOnly`, `Write`, `Exec` or
`Destructive` and `segments(line)` lists the simple commands it runs, both from
one tree-sitter-bash walk. It parses and walks only; it never runs anything.

The classifier is a trust guard. A line it cannot parse, a substitution, an
`eval` or `sh -c` string and a variable assignment other than a short safe list
stay at least `Exec` and make the segments opaque, so no allow rule can cover
them. A change that narrows this needs a regression test in `tests/classify.rs`.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
