# file-backup

Copy a file to `<name>.bak-<unix-seconds>` beside it.

## Cloud review findings (2026-10-08)

New bugs, dead code and moves for `file-backup` from a read-only Cursor cloud review of crates-packages `main` at `eaf543e` (agent `bc-e41445c4-afae-5c7d-a3c3-d96640037ad5`; full report: `cloud/crates.md` in the private `listepo/roadmap` repo). They take ids T2–T4, ordered P0, P1, P2. **confirmed** means seen in the tree or reproduced; **suspected** means plausible from the code but not proven (nothing was run on Windows or macOS). Line numbers are as of the review. None of these is in the task table yet: to take one, add its row the usual way.

| ID | Priority | Kind | Status | Where | Fix |
| --- | --- | --- | --- | --- | --- |
| T2 | P1 | bug | confirmed (reproduced) | `file-backup/src/lib.rs:34-35`, `:50-64` | A `.bak-*` sibling that is a hard link to the source counts as an existing backup, so `backup_at` returns `None`; overwriting the source then changes the "backup" too, and the only copy is lost. Skip only when the sibling is a distinct regular file (compare device and inode), or refuse links. |
| T3 | P2 | bug | confirmed | `file-backup/src/lib.rs:20-22` | A dangling source symlink counts as missing (`Path::exists` is false), so `backup` returns `Ok(None)` and the caller can replace the link with no copy. Use `symlink_metadata`; back up the link or return an error. |
| T4 | P2 | dead code | confirmed | `file-backup/done.md:7-10` | T1's record describes a `dunnage` just recipe; the `justfile` calls `swarfr`. Correct the record. |

Related, filed elsewhere: the repo-wide Sonar, CodeRabbit and test-fixture items are in `scoped-check/plan.md` (T5–T7).

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
