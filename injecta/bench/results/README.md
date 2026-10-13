Local measurements, 2026-10-08, Apple M3 Max (16 cores, 64 GB), macOS 27.0.1, rustc 1.99.0.
bench-results*.txt: `cargo bench -p di-bench --bench resolve -- --sample-count 500` (two runs).
build-results.csv: measure.sh RUNS=3; -2: RUNS=5; -3: RUNS=5 with CPU time columns.
Load average was 280-375 during build runs (other processes); wall-clock build times are noisy.
criterion-2026-10-08.md: `cargo bench -p criterion-bench --bench criterion -- --noplot`, then `criterion-bench/report.py` (medians). Load average 58 at the start and 468 at the end of the run.
perf-before.txt / perf-after.txt: `before-after.sh` on the commit before and after the `get` change (the "after" run hit load average 55-130; see docs/performance.md).
