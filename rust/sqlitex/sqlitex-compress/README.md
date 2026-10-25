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
