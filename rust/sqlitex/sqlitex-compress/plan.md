# sqlitex-compress — plan

Algorithm-agnostic compression for SQLite BLOB data, with brotli, zstd and lz4
behind optional Cargo features. This document describes the crate as
implemented on `feat/sqlitex-compress` (PR #124) and what remains.

## Name and location

- Crate: `sqlitex-compress` (`rust/sqlitex/sqlitex-compress`).
- Lives in the `sqlitex` family next to `sqlitex-vector` and `sqlitex-graph`
  (`rust/sqlitex/sqlitex-vector`, `rust/sqlitex/sqlitex-graph`); those siblings are
  planned but not yet in this repo, so `sqlitex-compress` is the first member.
- Edition 2021, MSRV 1.86, `unsafe_code = "forbid"`, workspace license.

## Goals

- One small trait (`Compressor`) that callers use to compress and decompress
  BLOBs before writing them to / after reading them from SQLite.
- Self-describing blobs: the first byte names the algorithm, so a reader can
  decompress without extra column metadata.
- Pay only for what you use: every algorithm is an opt-in feature; with no
  features the crate compiles only the trait, the error type and the header
  helpers.
- Pure Rust where possible (`lz4_flex`, `brotli`); `zstd` uses the C library via
  the `zstd` crate.

## Non-goals (for now)

- No SQL functions (no `compress()`/`decompress()` registered in SQLite); the
  crate does not depend on `rusqlite` or any SQLite binding.
- No dictionaries (zstd/brotli trained dictionaries).
- No streaming API; whole blobs in memory only.
- No automatic algorithm selection or "store uncompressed if bigger" logic.

## The `Compressor` trait

Quoted from `src/lib.rs`:

```rust
/// A compression algorithm for SQLite BLOB data.
///
/// `compress` output always starts with [`Compressor::algorithm`]'s tag byte;
/// `decompress` checks it and fails with [`Error::TagMismatch`] otherwise.
pub trait Compressor {
    /// Algorithm this compressor implements.
    fn algorithm(&self) -> Algorithm;
    /// Compress `data`, returning a tagged blob.
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>, Error>;
    /// Decompress a tagged blob produced by [`Compressor::compress`].
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, Error>;
}
```

The trait is object safe (`&dyn Compressor` is used in tests). Supporting API:

- `Level` — `Fastest`, `Default` (default), `Best`, `Custom(i32)`; each
  algorithm maps/clamps it to its own range.
- `split_header(data) -> Result<(Algorithm, &[u8]), Error>` and
  `expect_header(data, expected) -> Result<&[u8], Error>`.
- Free function `decompress(data)` — reads the tag and dispatches to the default
  compressor for it, or returns `Error::AlgorithmDisabled` if that feature is off.

## Features

```toml
[features]
default = []
brotli = ["dep:brotli"]
zstd = ["dep:zstd"]
lz4 = ["dep:lz4_flex"]
```

`default = []`: nothing is linked into the binary unless the feature is enabled.

| Feature | Crate | Type | Levels |
|---|---|---|---|
| `brotli` | `brotli` 8 | `brotli::Brotli { level }` | quality: fastest 0, default 6, best 11, custom clamped 0..=11; window 22, buffer 4096 |
| `zstd` | `zstd` 0.13 | `zstd::Zstd { level }` | fastest 1, default 3, best 19, custom clamped 1..=22 |
| `lz4` | `lz4_flex` 0.11 | `lz4::Lz4` | none (lz4_flex has no levels); block format with prepended size |

## Payload format

```
+--------+-----------------------------+
| tag u8 | algorithm-specific payload  |
+--------+-----------------------------+
```

| Tag | Algorithm | Payload |
|---|---|---|
| `0x01` | brotli | brotli stream |
| `0x02` | zstd | zstd frame (`zstd::bulk::compress`) |
| `0x03` | lz4 | `lz4_flex::compress_prepend_size` (u32 LE size + block) |

Tags are stable on-disk values and must never be reused. `0x00` and `0x04..=0xFF`
are unassigned.

## Error handling

`Error` (implements `Display` and `std::error::Error`):

- `MissingHeader` — empty input, no tag byte.
- `UnknownTag(u8)` — tag is not 0x01/0x02/0x03.
- `AlgorithmDisabled(Algorithm)` — known tag, but its feature is not compiled in
  (from the free `decompress`).
- `TagMismatch { expected, found }` — a specific compressor was given another
  algorithm's blob.
- `Codec { algorithm, message }` — the codec failed (corrupt data, codec I/O).

No panics on malformed input; all paths return `Result`.

## Tests

`tests/roundtrip.rs`:

- Round-trip of empty, small, repetitive and pseudo-random inputs for every
  enabled algorithm and every `Level`.
- Header errors: empty input, unknown tag.
- `tag_mismatch` (needs `zstd` + `lz4`).
- `disabled_algorithm` (only without `zstd`): a zstd-tagged blob returns
  `AlgorithmDisabled`.

Feature matrix — all 8 combinations must build, pass tests and clippy:

| | brotli | zstd | lz4 |
|---|---|---|---|
| none | | | |
| brotli | x | | |
| zstd | | x | |
| lz4 | | | x |
| brotli+zstd | x | x | |
| brotli+lz4 | x | | x |
| zstd+lz4 | | x | x |
| all | x | x | x |

Local: `cargo hack test -p sqlitex-compress --feature-powerset` (or a loop over
`--no-default-features --features …`).

### CI

The workspace CI run only builds default features (= none), so `ci.yml` needs a
step that runs the feature powerset for `sqlitex-compress` (test + clippy
`-D warnings`), and a `cargo tree` check that the no-feature build links none of
`brotli`, `zstd`, `lz4_flex` (same pattern as the `llm-http` step). Not added yet.

## Docs

- Crate-level rustdoc in `src/lib.rs`, item docs on all public items.
- `README.md` with feature table, payload format and a usage example.
- docs.rs: build with `all-features = true` so every module is documented.

## Limitations

- Whole-buffer, in-memory only; large blobs cost 2x memory.
- One tag byte overhead; compressed output can be larger than input for
  incompressible data — caller decides whether to store raw.
- `zstd` pulls a C library (build needs a C toolchain; affects wasm targets).
- No format version beyond the tag; changing a codec's payload layout requires
  a new tag.

## Future work

- SQL functions behind a `rusqlite` feature (`compress(blob, 'zstd')`,
  `decompress(blob)`).
- zstd/brotli dictionaries (likely new tags or a dictionary id after the tag).
- Streaming `Read`/`Write` adapters.
- "Store raw if not smaller" helper with its own tag.
- Benchmarks (criterion) per algorithm/level on representative blobs.
- Integration with `sqlitex-vector` / `sqlitex-graph` storage.

## Open questions

- Reserve `0x00` for "uncompressed"?
- Should `decompress` allow a size limit to guard against decompression bombs?
- Pure-Rust zstd (`ruzstd`) for decompress-only / wasm builds?
- Is MSRV 1.86 aligned with the rest of the workspace?
- Publish to crates.io now or after SQL functions land?
