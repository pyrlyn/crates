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

## Scalar mode (proposed, feature `scalar`)

Status: plan only. **Scope change:** the non-goals above exclude SQL functions;
two of the interception options below need them. Whether to lift that non-goal
is an open question (see the end of this section).

A second, independent mode: compress TEXT and numeric values at insert time and
store them as tagged BLOBs, so callers keep writing ordinary strings/numbers.
Behind its own feature `scalar`, off by default, independent of the blob mode
(see "Modes, features and the zero state" for the flag layout).

### Intercepting inserts

| Option | How | Pros | Cons |
|---|---|---|---|
| A. Rust encode API | `scalar::encode(&Value) -> Vec<u8>` / `decode(&[u8]) -> Value`, plus an optional insert wrapper over the host's statement API | no SQL surface, no new deps, works with any driver, testable in isolation | every writer/reader must go through Rust; raw SQL clients see BLOBs |
| B. App-defined function + `BEFORE INSERT` trigger | register `sqlitex_compress(value, alg)`; trigger rewrites `NEW.col` | transparent for SQL writers | SQLite `BEFORE` triggers cannot modify `NEW`; needs an `AFTER INSERT ... UPDATE` (double write) — awkward and slow |
| C. View + `INSTEAD OF` triggers | users write to `v_logs`; triggers call `sqlitex_compress()` into the base table; the view selects `sqlitex_decompress(col)` | transparent reads and writes in SQL | needs SQL functions; views hide indexes; more schema to migrate |
| D. Virtual table | `CREATE VIRTUAL TABLE logs USING sqlitex_compress(...)` | full control of storage and reads | largest implementation and `unsafe`/FFI surface, conflicts with `unsafe_code = "forbid"` |

**Recommendation:** ship A first (pure Rust, fits the current non-goals). C is the
follow-up if SQL-level transparency is required; it depends on the open question
about SQL functions. B and D are not planned.

### Stored format

The column holds a BLOB: `[algorithm tag][type tag][payload]`.

- Algorithm tag: same values as the blob mode (`0x01` brotli, `0x02` zstd,
  `0x03` lz4), plus **`0x00` = passthrough** (stored uncompressed).
- Type tag restores the SQLite type on decode: `0x01` TEXT (UTF-8), `0x02`
  INTEGER (i64 LE), `0x03` REAL (f64 LE), `0x04` BLOB. NULL is never encoded,
  it stays NULL.
- Minimum-size threshold (default 64 bytes, configurable): values below it, or
  values that do not shrink, are stored with tag `0x00`. Header cost is then
  2 bytes, never a compressed frame.
- Numbers are almost never worth compressing (8 bytes); they go through
  passthrough unless explicitly forced. Supporting them is about one uniform
  column format, not about saving space.

### Tradeoffs

- Every read decodes (and usually decompresses) the value.
- Lost on encoded columns: indexes that mean anything, range queries,
  `LIKE`/`GLOB`, `ORDER BY`, `GROUP BY` on content, type affinity, `typeof()`
  returning the logical type, and FTS.
- Small values grow by the 2-byte header (passthrough) or by a codec frame
  (prevented by the threshold).
- Raw SQL tools (sqlite3 CLI, DB browsers) show opaque BLOBs.

**Use it for:** large TEXT that is written once and read rarely — logs, JSON
event payloads, rendered documents, audit trails.
**Don't use it for:** keys, foreign keys, numbers, anything in `WHERE`/`ORDER BY`,
short strings, columns that need FTS.

### Difference from the blob mode

| | blob mode | scalar mode |
|---|---|---|
| Input | `&[u8]` the caller already treats as a BLOB | TEXT/INTEGER/REAL/BLOB values |
| Header | 1 byte (algorithm) | 2 bytes (algorithm + type) |
| Passthrough | no (always compressed) | yes, tag `0x00` below threshold |
| Restores type | n/a | yes |
| Feature | `blob` | `scalar` |

The two formats are distinct; a scalar value is not a valid blob-mode payload
(tag `0x00` and the type byte), and decoders return `UnknownTag` /
`TagMismatch`-style errors rather than guessing.

### Tests

- Round-trip of TEXT (ASCII, multibyte UTF-8, empty), INTEGER (0, ±1,
  `i64::MIN/MAX`), REAL (0.0, -0.0, NaN bits, ±inf), BLOB for every enabled
  algorithm.
- Threshold: 63/64/65 bytes; incompressible values fall back to passthrough.
- Type tag errors: unknown type tag, truncated payload, invalid UTF-8 → errors,
  no panics.
- Scalar payload rejected by blob-mode decode and vice versa.
- `scalar` with no algorithm: everything is passthrough (see below).

Feature matrix: 3 algorithms × 2 modes = 2⁵ = 32 combinations. CI runs all of
them for `cargo test` + `cargo clippy -D warnings` (cheap: one crate). If that
gets slow, CI runs the 16 combinations with `blob` on plus the zero state and
`scalar`-only with each algorithm.

### Open questions (scalar)

- Lift the "no SQL functions" non-goal for option C (likely behind a separate
  `sql` feature with `rusqlite`)?
- Default threshold value; per-column configuration?
- Should numbers be accepted at all, or rejected with an error?

## Modes, features and the zero state (proposed)

### Flags

- Algorithm features: `brotli`, `zstd`, `lz4`.
- Mode features: `blob` (current API: `Compressor`, `decompress`) and `scalar`.
- `default = []`: everything off.
- Today the blob API is always compiled; moving it behind `blob` is a breaking
  change for 0.x and lands together with `scalar`.

### Zero state (no features)

The crate (and the SQLite extension, once SQL registration exists) still builds
and loads, but is a no-op:

- Registered: only a status function, e.g. `sqlitex_compress_version()` returning
  the crate version and the enabled features (`"0.1.0;features="`). In the Rust
  API: `sqlitex_compress::enabled_features() -> &'static [&'static str]`.
- No compress/decompress functions, no triggers, no virtual tables are
  registered; writes and reads pass through untouched (the extension never sees
  the data).
- Decoding an existing tagged value in the zero state returns
  `AlgorithmDisabled` / `ModeDisabled`, never corrupts or silently returns bytes.

### Mode without an algorithm

Options: `compile_error!` when `blob` or `scalar` is on without any algorithm,
or a runtime passthrough.

**Recommendation: runtime passthrough, no compile error.** Cargo features are
additive and unified across the dependency graph, so a `compile_error!` would
break a build where one crate enables `scalar` and another picks the
algorithm. Behaviour:

- `scalar` without algorithms: every value is stored with tag `0x00`
  (passthrough) and decodes normally.
- `blob` without algorithms: there are no `Compressor` implementations; the
  free `decompress` returns `AlgorithmDisabled` (today's behaviour).
- A `cfg`-gated warning in the docs and `enabled_features()` make the state
  visible.

### Size and dependency guarantees

- No features: no codec crates in the normal dependency tree (CI's
  `cargo tree` check already enforces `brotli`, `zstd`, `lz4_flex`; extend it to
  any future `sql`/`rusqlite` dependency).
- No features: only `std`; the compiled crate adds only the status function and
  error types (target: a few KB in a release cdylib; measured in CI by
  comparing the `.rlib`/cdylib size against a recorded baseline, warning only).
- Each algorithm feature adds exactly one codec crate.

### Zero-state test

- `tests/zero.rs`, compiled only with `cfg(not(any(feature = "brotli",
  feature = "zstd", feature = "lz4", feature = "blob", feature = "scalar")))`:
  `enabled_features()` is empty, the version string reports no features,
  decoding any tag returns the disabled error, and (once SQL registration
  exists) opening an in-memory DB with the extension and inserting/selecting
  TEXT, INTEGER, REAL and BLOB returns the values unchanged.
- CI: the `cargo tree` check for the no-feature build, plus the size baseline.

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
