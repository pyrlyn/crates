# target-prune (Rust)

Keeps a Cargo `target/` dir from bloating: it keeps the artifacts of the last `--keep N` build
runs **plus everything they transitively depend on**, and deletes the rest.
Rust rewrite of an earlier Python prototype, same algorithm. **Dry run by default**;
nothing is deleted without `--apply`.

## How it works
1. Finds every Cargo profile dir (any dir with a `.fingerprint/`: `target/debug`,
   `target/<triple>/release`, ...) under the given roots.
2. Each `.fingerprint/<name>-<hash>/` is a *unit* (lib, bin, test binary, build script, build
   script run). Its artifacts are the entries of `deps/`, `build/`, `incremental/`, `examples/`
   carrying the same hash (`libfoo-<hash>.rlib/.rmeta/.dylib`, `foo-<hash>.d`, test executables
   `foo-<hash>`, `build/foo-<hash>/`).
3. Dependency graph: each unit's `*.json` fingerprint lists deps as `(name, fingerprint u64)`;
   each unit's plain fingerprint file holds its own u64 (hex). Matching them gives the edges.
4. Build runs: units sorted by their newest fingerprint mtime. A new run starts when
   `target-prune run` recorded a build start in between (`target/.target-prune-runs`), or
   after a quiet gap longer than `--gap` seconds (default 60) when there are no markers.
5. Keep = units of the last N runs + transitive closure. If a dep fingerprint moved on, every
   unit of that crate name is kept (never guesses).
6. Delete: every other unit's files; all but the newest session (and its `.lock`) of each kept
   `incremental/` dir; whole `incremental/<crate>-<id>` dirs (named by rustc's crate id, which
   usually matches no unit hash) not touched since the oldest kept run started.
7. Smart rule (on by default, `--no-smart` turns it off): inside the kept runs, a unit whose
   only output is an executable nothing depends on (a test harness or bin in `deps/`) is
   dropped when a newer kept unit of the same name exists. A config change (features, debug,
   RUSTFLAGS) produces a full new set of test binaries; the old set is the biggest single item
   in `target/` and is never run again until that config comes back. The libraries those old
   binaries were linked from stay (they are in the closure), so switching back is a relink,
   not a rebuild.
8. Size budget (`--max-size 12G`, optional): if what the plan keeps is still larger than the
   budget, the plan is redone with one run fewer, down to `--keep 1`. The newest run is never
   dropped to meet the budget, so the budget is a target, not a hard cap.
9. Fail safe: if a kept unit has a dep that maps to no unit at all, that profile dir is left
   untouched.

Uplifted files (`target/debug/<bin>`, `.d` next to it) and anything outside those dirs are
never touched.

## Build / install
```sh
cargo build --release            # binary: target/release/target-prune
cargo install --path .           # optional: puts target-prune on PATH
```

## Use
```sh
target-prune prune                          # dry run over the default root
target-prune prune path/to/target -v        # dry run, list every path
target-prune prune --keep 3 --apply PATH    # delete for real
target-prune run --keep 2 --apply -- cargo build      # build, then prune ./target
target-prune run --apply -- cargo test --no-run       # $CARGO_TARGET_DIR is honoured
target-prune run --keep 2 --max-size 12G --apply -- cargo test --no-run   # with a budget
target-prune prune --no-smart PATH          # old behaviour: keep every exe of kept runs
target-prune snapshot target                # JSON per profile: sizes, dup hashes, sessions
```
`run` appends the start time to `<target>/.target-prune-runs`, runs the command, and prunes
only if it exited 0 (exit code is passed through). Prune time goes to stderr.

## Enable / disable
- Enable per project: alias the build, e.g. a `just`/mise task
  `build: target-prune run --apply -- cargo build`, or a shell alias
  `alias cb='target-prune run --apply -- cargo build'`. Cargo has no post-build hook, so a
  wrapper (or a cron/launchd job running `prune --apply <root>`) is the way to attach it.
- Disable: drop the alias/task/job. Optionally delete `target/.target-prune-runs`; without it
  the tool falls back to `--gap` grouping. Nothing else is installed.

## Test
```sh
cargo test
```
Fixtures are synthetic target dirs created under `./tmp/` and removed after each test.
`bench/bench.sh A|B` is the rtok benchmark (only for a scratch worktree).

## Limitations
- Cargo's on-disk layout is unstable; unknown layouts end in the fail-safe skip.
- A unit cargo found fresh is not re-stamped; it stays alive only through the closure from a
  kept run. Alternating between configurations (features, RUSTFLAGS, profiles) more than N
  ways rebuilds the dropped configuration from scratch next time.
- Without markers, two builds closer than `--gap` merge into one run (keeps more, not less).
- Smart rule: going back to a config whose test binaries were dropped relinks them (seconds
  to a minute per binary, no recompilation). Use `--no-smart` if you alternate between two
  configs on every build and link time matters more than disk.
- `--max-size` only lowers `--keep`; one run larger than the budget stays as it is.
- Do not run `--apply` while another cargo is building in the same target dir.
- Nightly cargo has its own `cargo clean gc` (`-Zgc`) that tracks last use in a DB; this tool
  does not need nightly.

## Benchmark results
See [bench/RESULTS.md](bench/RESULTS.md) (rtok, 10 runs with and without pruning).
