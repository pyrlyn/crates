# scoped-check

Run only the check commands a change touches. It computes the paths changed against a
base ref, maps them to gates by glob, and narrows each gate's command to the affected cargo
packages.

```bash
cargo install --path scoped-check
scoped-check plan   # what would run, and why
scoped-check run    # run it
```

## Config: `scoped-check.toml` at the repository top level

```toml
base = "origin/main"   # default; --base overrides
unmatched = "all"      # path-gates key: a path no gate claims selects every gate
workspace = "."        # cargo workspace root, relative to this file

[[gate]]
name = "fmt"
paths = ["**/*.rs"]
run = "cargo fmt --all -- --check"

[[gate]]
name = "test"
paths = ["**/*.rs", "tests/**"]
run = "cargo nextest run {packages} {nextest_filter}"

[[gate]]
name = "docs"
paths = ["**/*.md"]
run = "just docs-check"
```

`paths`, `always` and `unmatched` are read by [`path-gates`](../path-gates). `run` is
executed with `sh -c` (`cmd /C` on Windows) from the config file's directory. A gate
without `run`, or a `run` with an unknown `{placeholder}`, is a config error (exit 2).
Only `{identifier}` is a placeholder; `${VAR}`, `awk '{print}'` and `{a,b}` stay literal.

| Placeholder | Scoped value | `--all` or `all: true` |
| --- | --- | --- |
| `{packages}` | `-p a -p b` (sorted, affected packages incl. reverse dependencies) | `--workspace` |
| `{nextest_filter}` | `-E 'package(=a) \| package(=b)'` | empty |
| `{changed}` | the paths the gate claims, shell-quoted | every changed path, shell-quoted |

Values are POSIX shell-quoted, so avoid `{changed}` on Windows. A gate using `{packages}` or
`{nextest_filter}` whose affected set is empty is skipped: nothing it covers changed. When
every gate runs, a gate using `{changed}` is skipped if there is no path to substitute
(nothing changed, or `--all` / the fallback could not compute the change set), so a command
like `gofmt -w {changed}` never runs with an empty argument list.

## Commands

- `scoped-check plan [--base REF] [--config PATH] [--all] [--json]` prints the base, merge
  base, changed-path count and every selected gate with its expanded command and why:
  `scoped`, `all: unmatched <path>`, `all: fallback after error: <error>` or `--all`.
- `scoped-check run [--base REF] [--config PATH] [--all] [--keep-going]` runs the selected
  gates in config order, printing `== <gate>: <command>` before each. It stops at the first
  failure unless `--keep-going`, which runs the rest and prints `N gate(s) failed: ...`.

Shared flags: `--fallback-all` (see below), `--no-cache` (do not read or write the cargo
metadata cache), `-v/--verbose` (say on stderr whether `cargo metadata` ran). `--config`
defaults to `scoped-check.toml` at the git top level of the current directory.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success: every selected gate passed, `plan` printed, or nothing changed |
| the gate's code | A gate failed; the first failing gate's code, or 1 when it has none (killed by a signal). The same under `--keep-going`, which only keeps running |
| 2 | Config or usage error (bad `scoped-check.toml`, unknown flag, no config file) |
| 3 | Planning error: not a git repository, git missing, unknown base ref, no merge base (including a shallow clone), `cargo metadata` failed. No gate ran |

A gate that itself exits 2 or 3 is indistinguishable from those codes by number alone; the
messages on stderr tell them apart.

## Errors are errors

Any error computing the change set or the affected packages prints `scoped-check: error:
<why>` on stderr and exits 3. Nothing is silently widened to "run everything", because a
warning plus exit 0 hides a broken pipeline. Intentional "all" decisions are not errors:
`--all`, a path no gate claims with `unmatched = "all"`, and a change to `Cargo.toml`,
`Cargo.lock`, `.cargo/config*`, `rust-toolchain*` or `build.rs`. `--all` ignores the change
set, so it also works when the change set cannot be computed.

A shallow clone usually has no merge base with the base ref. The error then says so and
suggests `fetch-depth: 0` for `actions/checkout`, `git fetch --unshallow` or
`git fetch --deepen=<n>`.

Opt-in fallback, off by default: `--fallback-all` or `fallback = "all"` in the config makes a
planning error run every gate instead. It prints a `WARNING: planning failed (...)` line on
stderr, appends the same line to `$GITHUB_STEP_SUMMARY` when that is set, and then exits by
the gates' results. `fallback = "error"` is the default.

## Speed: `cargo metadata`

Planning time is almost all `cargo metadata`. It runs at most once per run (one graph for
every gate) and not at all when no selected gate uses `{packages}` or `{nextest_filter}`,
when the change set alters the build (manifests, lock file, toolchain: all packages are
selected anyway), or when `--all` is given.

The JSON is also cached between runs in `$SCOPED_CHECK_CACHE_DIR`, else
`$XDG_CACHE_HOME/scoped-check`, else `~/.cache/scoped-check` (one file per workspace root).
The key hashes the content of `Cargo.lock`, every `Cargo.toml` under the workspace,
`.cargo/config*`, `rust-toolchain*` and the output of `cargo --version`. Path dependencies
outside the workspace are re-hashed on every hit. Any doubt (no `Cargo.lock`, an unreadable
file, a corrupt entry, a different cargo) recomputes. Modification times are not used:
a checkout resets them. `--no-cache` turns it off; `-v` prints `cargo metadata: skipped (cache
hit)` or `ran (cache miss, stored)`. A cache that cannot be written is a warning, not an error.

Measuring: `scripts/bench.sh` compares `run --all` with `run` on your current change.

```bash
# From the repository root that has scoped-check.toml. It RUNS the real gates, twice.
path/to/scoped-check/scripts/bench.sh [BASE]   # default BASE: merge-base with origin/main
```

It prints each run's time and exit code, the difference and the percentage saved. A failing
gate is reported, not fatal to the script. `SCOPED_CHECK` names the binary if it is not on
`PATH`. Needs bash, git and one of bash 5, perl or python3 for the timer.

## `plan --json`

```json
{
  "base": "origin/main",
  "merge_base": "<sha>",
  "changed": 3,
  "nothing_changed": false,
  "gates": [{ "name": "test", "command": "cargo nextest run -p a -p b ...", "why": "scoped" }],
  "skipped": [{ "name": "docs", "reason": "nothing it covers changed" }],
  "error": null
}
```

`merge_base` and `changed` are `null` when the change set could not be computed. `error` is
the planning error, else `null`; with an error `plan --json` still prints the document, then
exits 3 (without `--fallback-all` `gates` is empty).
