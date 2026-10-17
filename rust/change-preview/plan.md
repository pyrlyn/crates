# change-preview

Preview what a command would change: unified diff or `--stat` of file edits, size and file count of removals, and a totals line.

## Cloud review findings (2026-10-08)

New bugs, dead code and moves for `change-preview` from a read-only Cursor cloud review of crates-packages `main` at `eaf543e` (agent `bc-e41445c4-afae-5c7d-a3c3-d96640037ad5`; full report: `cloud/crates.md` in the private `listepo/roadmap` repo). It takes id T2. **confirmed** means seen in the tree or reproduced; **suspected** means plausible from the code but not proven (nothing was run on Windows or macOS). Line numbers are as of the review. None of these is in the task table yet: to take one, add its row the usual way.

| ID | Priority | Kind | Status | Where | Fix |
| --- | --- | --- | --- | --- | --- |
| T2 | P2 | bug | confirmed (reproduced) | `change-preview/src/lib.rs:214-234`; example at `change-preview/README.md:20` | `paint` drops the trailing newline that `render` emits (`lines()` + `join("\n")`, so `paint("a\n") == "a"`), which breaks the README example. Re-append the newline when the input ends with one. |

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
