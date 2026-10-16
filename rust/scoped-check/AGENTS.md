# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

A binary that runs only the check commands a change touches. It glues three sibling
crates: `git-changed-paths` (change set), `path-gates` (glob rules to gates) and
`cargo-changed-packages` (affected workspace packages). Do not re-implement any of them.

## Rules

- Fail loudly: any error computing the change set or the affected packages is an error on
  stderr and exit 3, never "nothing to do" and never a silent `--all`. Running every gate
  after an error exists only as the opt-in `--fallback-all` / `fallback = "all"`, with a loud
  warning. A config error exits 2; a failing gate exits with its own code (also under
  `--keep-going`).
- `cargo metadata` runs once per run, lazily, through one `Resolver`; do not call
  `cargo_changed_packages::affected` per gate.
- Tests count `cargo metadata` runs with a `cargo` shim passed through `$CARGO`; new
  speed behaviour needs such a test.
- Substituted values are shell-quoted with `shlex`; the `run` text itself is the
  repository's own, trusted like its justfile.
- Tests build throwaway git repos with `tempfile`; keep host git config out of them.

## Commands

```bash
cargo test -p scoped-check -p cargo-changed-packages -p git-changed-paths
cargo clippy -p scoped-check --all-targets
cargo fmt
```
