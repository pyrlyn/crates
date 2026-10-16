# cargo-changed-packages

Workspace packages a set of changed paths affects, including reverse dependencies.

## Cloud review findings (2026-10-08)

A read-only Cursor cloud review of crates-packages `main` at `eaf543e` (full report: `cloud/crates.md` in the private `listepo/roadmap` repo) found no bugs, dead code or moves of this crate's own. Its test fixture in `cargo-changed-packages/tests/affected.rs:14-34` is one of three copies that `scoped-check/plan.md` T5 moves into a shared `crates-test-support` dev crate.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
