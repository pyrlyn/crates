# shell-classify — completed tasks

### T1. Extract the bash risk classifier from the coding agent

Moved out of the coding agent's `crates/agent-tools/src/bash/classify.rs` (aulo task T1.10): one tree-sitter-bash walk that gives `classify(line) -> Risk` (`ReadOnly`, `Write`, `Exec`, `Destructive`) and `segments(line) -> Segments` (the simple commands plus an `opaque` flag), with wrapper unwrapping (`nohup`, `timeout 5`, `env`, ...), `eval`/`sh -c` re-parsing, safe-assignment handling and the `curl ... | sh` rule. `Risk` and `Segments` are defined in the crate, and the bash parser setup (agent-syntax's `parse_bash`) is inlined, so it has no the coding agent dependency. No behaviour change; the coding agent classifier tests moved to `tests/classify.rs`. Check: `cargo test -p shell-classify`, `cargo clippy -p shell-classify --all-targets -- -D warnings` green on Rust 1.99.

### T26. Classify Git output and external diff flags by their effects

`git diff`, `git log`, and `git show` now classify as `Exec` when given `--output` or `--ext-diff`. The regression test covers joined and separate output flags and preserves ordinary read classifications. The test failed before the fix and passed after it; crate tests, clippy and formatting checks passed.
