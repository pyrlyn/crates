# scoped-check

Run only the check commands a change touches.

## Cloud review findings (2026-10-08)

New bugs, dead code and moves for `scoped-check` from a read-only Cursor cloud review of crates-packages `main` at `eaf543e` (agent `bc-e41445c4-afae-5c7d-a3c3-d96640037ad5`; full report: `cloud/crates.md` in the private `listepo/roadmap` repo). They take ids T2–T7, ordered P0, P1, P2. **confirmed** means seen in the tree or reproduced; **suspected** means plausible from the code but not proven (nothing was run on Windows or macOS). Line numbers are as of the review. T5–T7 cover the whole repository, which has no root plan; they are filed here because scoped-check composes the other crates. None of these is in the task table yet: to take one, add its row the usual way.

| ID | Priority | Kind | Status | Where | Fix |
| --- | --- | --- | --- | --- | --- |
| T2 | P2 | bug | suspected (Git for Windows not run) | `scoped-check/src/main.rs:101` | The toplevel path strips only `\n`, not `\r`. Use `.trim()`, or call `git-changed-paths` `toplevel()` (T3). |
| T3 | P2 | dead code | confirmed | `scoped-check/src/main.rs:95-102` | A second `rev-parse --show-toplevel` copy, without `-C` or `GIT_OPTIONAL_LOCKS`. Call `git-changed-paths` `toplevel(repo)` (`git-changed-paths/plan.md` T3) and delete this one. |
| T4 | P2 | dead code | confirmed | `scoped-check/src/config.rs:73-74` | The same TOML is parsed twice (`toml::from_str`, then `Rules::from_toml`). Parse once and build `Rules` from `GateSpec`s (`path-gates/plan.md` T3). |
| T5 | P2 | move | confirmed | `scoped-check/tests/cli.rs:66-91`; `git-changed-paths/tests/changed_paths.rs:40-54`; `cargo-changed-packages/tests/affected.rs:14-34` → a `crates-test-support` dev crate | Three copies of "init git, write a tiny workspace, write a member". Share them. |
| T6 | P2 | dead code | confirmed | `sonar-project.properties:6`; `README.md` badge | Sonar indexes only `change-preview/src` and `file-backup/src`, so four crates get no analysis, and the README badge still says `listepo_crates-packages`. List every crate's `src/` and fix the badge. |
| T7 | P2 | dead code | confirmed | `.coderabbit.yaml:4` | It names `.github/workflows/coderabbit-issues.yml`, which is not in the tree. Drop the reference or add the workflow. |

Not added: the open PRs #26 (`atomic-replace`), #29 (`app-home`) and #30 (`injecta`) bring crates into this repo; the review only confirmed that the PRs exist.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
