# path-gates

Map changed paths to named gates by glob rules.

## Cloud review findings (2026-10-08)

New bugs, dead code and moves for `path-gates` from a read-only Cursor cloud review of crates-packages `main` at `eaf543e` (agent `bc-e41445c4-afae-5c7d-a3c3-d96640037ad5`; full report: `cloud/crates.md` in the private `listepo/roadmap` repo). They take ids T2–T3, ordered P0, P1, P2. **confirmed** means seen in the tree or reproduced; **suspected** means plausible from the code but not proven (nothing was run on Windows or macOS). Line numbers are as of the review. None of these is in the task table yet: to take one, add its row the usual way.

| ID | Priority | Kind | Status | Where | Fix |
| --- | --- | --- | --- | --- | --- |
| T2 | P2 | bug | suspected | `path-gates/src/lib.rs:177-178` | Globs are case-sensitive (`GlobBuilder` without `case_insensitive`), which can miss paths on default macOS and Windows volumes. Document it, or add a case-insensitive option. |
| T3 | P2 | move | confirmed | scoped-check `Raw`/`RawGate` (`scoped-check/src/config.rs:73-74`) → `path-gates` (`GateSpec`) | Let `path-gates` build `Rules` from the parsed `[[gate]]` tables so the host parses the file once (`scoped-check/plan.md` T4). |

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
