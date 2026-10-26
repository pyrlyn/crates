# sqlitex-compress

Algorithm-agnostic compression for SQLite BLOB data. The core is a small
`Compressor` trait; brotli, zstd and lz4 are optional Cargo features.

## How it works

- `Compressor` has `algorithm()`, `compress(&[u8]) -> Result<Vec<u8>, Error>` and
  `decompress(&[u8]) -> Result<Vec<u8>, Error>`.
- Every compressed blob is `[tag byte][algorithm payload]`. Tags: `0x01` brotli,
  `0x02` zstd, `0x03` lz4 (`lz4_flex` block format with prepended size). The tag
  lets `sqlitex_compress::decompress` pick the algorithm, so a column can mix
  algorithms.
- `Level` (`Fastest`, `Default`, `Best`, `Custom(i32)`) configures brotli and zstd;
  each clamps it to its own range. lz4 has no levels.
- `Error`: `MissingHeader`, `UnknownTag`, `AlgorithmDisabled`, `TagMismatch`, `Codec`.

```rust,ignore
use sqlitex_compress::{zstd::Zstd, Compressor, Level};
let c = Zstd { level: Level::Best };
let blob = c.compress(b"row payload")?;      // store in a BLOB column
let raw = sqlitex_compress::decompress(&blob)?;
```

## Features

| Feature | Crate | Module |
| --- | --- | --- |
| `brotli` | `brotli` | `sqlitex_compress::brotli::Brotli` |
| `zstd` | `zstd` | `sqlitex_compress::zstd::Zstd` |
| `lz4` | `lz4_flex` | `sqlitex_compress::lz4::Lz4` |

**Default features: none.** Without a feature its module and dependency are not
compiled. Enable: `sqlitex-compress = { version = "0.1", features = ["zstd"] }`.
Disable: leave the feature out. Decompressing a blob whose algorithm is not
compiled in returns `Error::AlgorithmDisabled`.

## Tests

```bash
cargo test -p sqlitex-compress
cargo test -p sqlitex-compress --features zstd   # or brotli, lz4
cargo test -p sqlitex-compress --all-features
```

Roundtrip per feature (empty, small, large repetitive, random) and errors on
corrupt input, missing header, unknown tag and tag mismatch.

## Limitations

- Whole-blob, in-memory compression; no streaming, no zstd dictionaries yet.
- Not a loadable SQLite extension: no SQL functions are registered yet; call it
  from Rust before writing and after reading a BLOB.
- The tag byte format is unversioned and may change before 1.0.
- Not published; `scripts/prerelease.sh` only looks at `rust/<crate>/`, not
  `rust/sqlitex/<crate>/`.

## Testing

`tests/matrix.rs` runs under every feature combination (none, each alone, pairs, all):
round-trip of empty, small, 256 KB random and 8 MB inputs, the first-byte tag per
algorithm, missing/unknown tag errors, tag mismatch between algorithms and
`AlgorithmDisabled` for each algorithm that is compiled out. CI runs `cargo test` and
`cargo clippy -D warnings` for all 8 combinations:

```sh
for f in "" brotli zstd lz4 brotli,zstd brotli,lz4 zstd,lz4 brotli,zstd,lz4; do
  cargo test  -p sqlitex-compress --no-default-features --features "$f"
  cargo clippy -p sqlitex-compress --no-default-features --features "$f" --all-targets -- -D warnings
done
```

## Benchmarks

### Criterion (`benches/throughput.rs`)

Compress and decompress throughput for each enabled algorithm on compressible
(repeated JSON lines) and random data at 1 KB, 64 KB and 1 MB; set
`SQLITEX_BENCH_16MB=1` to add 16 MB. Algorithms are gated by `cfg(feature)`, so
enable the ones to measure:

```sh
cargo bench -p sqlitex-compress --all-features --bench throughput            # full
SQLITEX_BENCH_16MB=1 cargo bench -p sqlitex-compress --all-features --bench throughput -- --quick
```

### Realistic SQLite data (`bench/run.py`)

Python 3 stdlib only. Builds a temporary SQLite DB (users, JSON events, text posts,
log lines; scale with `--rows`), exports every text value as one row, and for the
baseline (`none`, rows copied as-is) and each algorithm runs `examples/dbbench.rs`
(reads `SQLITEX_BENCH_INPUT`, writes `SQLITEX_BENCH_OUTPUT`, algorithm from
`SQLITEX_BENCH_ALG`).

```sh
python3 rust/sqlitex/sqlitex-compress/bench/run.py --rows 5000            # small
python3 rust/sqlitex/sqlitex-compress/bench/run.py --rows 500000          # large
python3 rust/sqlitex/sqlitex-compress/bench/run.py --algs none,zstd
```

Metrics and how they are collected:

| Column | How |
|---|---|
| ratio | sum of raw row bytes / sum of compressed row bytes (tag byte included) |
| comp / decomp MB/s | raw MB / wall time of compressing (decompressing) all rows in the Rust driver, `std::time::Instant` |
| insert rows/s | rows / time of `executemany` INSERT of the driver's output into a fresh `blobs(id, data BLOB)` table plus `COMMIT`, measured in Python with `time.perf_counter` |
| db MB | `os.path.getsize` of that DB file after commit |
| peak RSS MB | `ru_maxrss` from `os.wait4` on the driver process (exact kernel peak) |
| avg RSS MB | mean of `ps -o rss=` samples of the driver every 10 ms; very short runs (the baseline) give few samples, so use a larger `--rows` for a meaningful average |

Everything is written to one `tempfile.mkdtemp()` directory (the cargo target too,
unless `CARGO_TARGET_DIR` is set), which is removed in a `finally` block on normal
exit, errors, Ctrl-C and SIGTERM. `bench/.gitignore` ignores any `*.db`/`*.bin`
in case of a stray copy.
