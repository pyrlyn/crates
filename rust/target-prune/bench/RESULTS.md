# Round 2 benchmark: rtok

`bench/bench.sh A|B` on a scratch rtok worktree (macOS, Apple Silicon). Each run makes a code
change plus a config change (debug info, features, RUSTFLAGS, dependency add/bump), then
builds including test binaries. Per run: `du-N.txt`, `snap-N.json` (deps/build/incremental
snapshot), and for series B `prune-N.log`. Raw tables: `out-A/table.tsv`, `out-B/table.tsv`.

## Series A: no pruning

| run | variant | build s | target MB |
|---|---|---|---|
| 1 | code | 424.1 | 10846 |
| 2 | code+debug=false | 161.4 | 18290 |
| 3 | code+nofeat-graph | 112.9 | 20994 |
| 4 | code+rustflags | 234.2 | 29168 |
| 5 | code+debug=false | 100.1 | 31525 |
| 6 | code+nofeat-graph+add-dep | 269.4 | 38915 |

Stopped after run 6: free disk fell below 30 GB. Growth is 3-8 GB per run, unbounded.

## Series B: `target-prune run --keep 2 --apply`

| run | variant | build s | target MB | freed | prune s |
|---|---|---|---|---|---|
| 1 | code | 310.1 | 10821 | 0 B | 1.79 |
| 2 | code+debug=false | 280.9 | 18070 | 0 B | 2.84 |
| 3 | code+nofeat-graph | 128.8 | 17848 | 4.1 GB | 3.57 |
| 4 | code+rustflags | 312.2 | 18995 | 9.7 GB | 9.59 |
| 5 | code+debug=false | 183.6 | 19699 | 7.3 GB | 11.41 |
| 6 | code+nofeat-graph+add-dep | 153.8 | 18319 | 10.7 GB | 7.85 |
| 7 | code+debug=false | 160.2 | 18941 | 6.4 GB | 4.82 |
| 8 | code+rustflags | 409.0 | 19826 | 10.1 GB | 23.25 |
| 9 | code+nofeat-graph+bump-dep | 338.9 | 21715 | 6.6 GB | 19.71 |
| 10 | code+debug=false | 175.6 | 18208 | 11.8 GB | 10.80 |

All 10 runs passed. `target/` settles at 18-20 GB and stops growing; builds are not slower.

## Growth analysis
- Integration test binaries dominate: 138 test files, each linked into its own binary of up to
  ~200 MB, about 8.2 GB per configuration.
- `incremental/` adds about 3.6 GB per run.
- Every change of flags, features or dependencies produces a full second set of the above, so
  `--keep 2` holds two configurations: 18-20 GB.
- `--keep 1` floor is about 9.5 GB (one configuration).

## Levers to get to 2-3 GB (not done here, project-level decisions)
- Merge integration tests into a single test binary (`tests/all/main.rs` with modules).
- `incremental = false` for tests / in CI.
