# shell-classify — completed tasks

### T1. Extract the bash risk classifier from cox

Moved out of cox's `crates/cox-tools/src/bash/classify.rs` (aulo task T1.10): one tree-sitter-bash walk that gives `classify(line) -> Risk` (`ReadOnly`, `Write`, `Exec`, `Destructive`) and `segments(line) -> Segments` (the simple commands plus an `opaque` flag), with wrapper unwrapping (`nohup`, `timeout 5`, `env`, ...), `eval`/`sh -c` re-parsing, safe-assignment handling and the `curl ... | sh` rule. `Risk` and `Segments` are defined in the crate, and the bash parser setup (cox-syntax's `parse_bash`) is inlined, so it has no cox dependency. No behaviour change; the cox classifier tests moved to `tests/classify.rs`. Check: `cargo test -p shell-classify`, `cargo clippy -p shell-classify --all-targets -- -D warnings` green on Rust 1.99.
