# scoped-check — completed tasks

### T1. Run only the checks a change touches

`scoped-check plan` and `scoped-check run` read `scoped-check.toml`, compute the paths a
change touched with `git-changed-paths`, map them to gates with `path-gates`, and expand
`{packages}`, `{nextest_filter}` and `{changed}` in each gate's command from
`cargo-changed-packages`. A config error exits 2.

### T2. Strict planning errors and a faster `plan`

Planning errors (git, base ref, merge base, shallow clone, `cargo metadata`) exit 3 with a
message instead of warning and running every gate; `plan --json` has an `error` field. The old
behaviour is the opt-in `--fallback-all` / `fallback = "all"`, with a loud warning and a
`$GITHUB_STEP_SUMMARY` line. `--keep-going` exits with the first failing gate's code. `cargo
metadata` runs once per run through one `Resolver` and is cached on disk by content hash
(`--no-cache`, `-v`). `scripts/bench.sh` times `run --all` against `run`.
