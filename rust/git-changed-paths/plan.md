# git-changed-paths

Paths a git working tree changed relative to a base ref.

## Cloud review findings (2026-10-08)

New bugs, dead code and moves for `git-changed-paths` from a read-only Cursor cloud review of crates-packages `main` at `eaf543e` (agent `bc-e41445c4-afae-5c7d-a3c3-d96640037ad5`; full report: `cloud/crates.md` in the private `listepo/roadmap` repo). They take ids T2–T3, ordered P0, P1, P2. **confirmed** means seen in the tree or reproduced; **suspected** means plausible from the code but not proven (nothing was run on Windows or macOS). Line numbers are as of the review. None of these is in the task table yet: to take one, add its row the usual way.

| ID | Priority | Kind | Status | Where | Fix |
| --- | --- | --- | --- | --- | --- |
| T2 | P2 | bug | suspected (Git for Windows not run) | `git-changed-paths/src/lib.rs:85` vs `:111` | The toplevel path strips only `\n`, not `\r` (the merge-base already uses `.trim()`). Use `.trim()` for both. The same fix in scoped-check is `scoped-check/plan.md` T2. |
| T3 | P2 | move | confirmed | `git-changed-paths/src/lib.rs:78-86` | Export the toplevel lookup as `toplevel(repo)` so scoped-check can drop its second copy (`scoped-check/plan.md` T3). |

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
