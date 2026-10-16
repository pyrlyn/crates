# shell-classify

Classify a bash command line by risk (read-only, write, exec, destructive) and list the simple commands it runs, with tree-sitter.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |

### T2. Adopt shell-classify in cox (needs publication)

Replace `crates/cox-tools/src/bash/classify.rs` in cox with this crate and delete the copy, together with `parse_bash` in `cox-syntax`. cox maps `shell_classify::Risk` and `Segments` to its own `cox_protocol` types (those derive serde and schemars, which this crate does not). Blocked until the crate is published: it is `publish = false` today, so the crate must first be published (or the release flow set up for it).
