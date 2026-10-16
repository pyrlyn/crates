# workspace-graph — completed tasks

### T1. Load the workspace graph and assert rules over it

Extracted from the dependency-direction tests that Scull (`crates/scull-ffi/tests/deps.rs`), cox (`crates/cox/tests/deps.rs`), aulo (`crates/aulo/tests/deps.rs`) and weft (`crates/weft-cli/tests/deps.rs`) each carried. `Graph::load` runs `cargo metadata --no-deps` and keeps the declared dependencies of the chosen `Kind`s per workspace member, external packages included; `workspace_only` drops the external edges. Rules: `check_only_dependents`, `check_forbidden`, `check_exact`, `check_layers`, each returning `Violations` and each with an `assert_*` form that panics with one line per violation. `FromIterator` builds synthetic graphs. Scull switched to it. Check: `cargo test -p workspace-graph` (10 tests) and clippy green; builds on Rust 1.98.
