# git-changed-paths — completed tasks

### T1. Changed paths relative to a base ref

`changed_paths(repo, base)` returns the merge-base sha and the repository-relative paths
changed by commits since the merge base, staged and unstaged edits, untracked files that
are not ignored, deletions and both sides of a rename. Uses the `git` CLI with `-z`
output; a failure is an `Error`, never an empty answer.

### T2. Shallow clone has its own error

When `git merge-base` finds nothing in a shallow clone the error is `ShallowNoMergeBase`,
whose message suggests `git fetch --unshallow` / `--deepen` or `fetch-depth: 0`.
