# sqlite-hnsw

Approximate nearest-neighbour search for embeddings: an HNSW graph kept in
memory for queries and persisted to a single SQLite file, with optional `i8`
quantization (about 4x less disk and memory than `f32`).

```rust,no_run
use sqlite_hnsw::{Params, Quant, VecDb};

let db = VecDb::open("vectors.db", 384, Params { m: 32, ef_construction: 400, ef_search: 1024, quant: Quant::I8, ..Params::default() })?;
db.insert(42, &embedding)?;          // any thread, concurrently
db.save()?;                          // writes only pages touched since the last save
for (id, score) in db.search(&query, 10) { /* cosine similarity, best first */ }
# Ok::<(), sqlite_hnsw::Error>(())
```

## How it works

See [docs/how-it-works.md](docs/how-it-works.md). In short: HNSW with the
neighbour-selection heuristic, cosine distance on normalized vectors, nodes in
a lock-free append-only arena, one `RwLock` per adjacency list (searches take
no global lock), and 1024-node BLOB pages in one SQLite table.

## Enable / disable

| feature | default | what it does |
|---|---|---|
| `bundled` | on | compiles SQLite in (`libsqlite3-sys/bundled`); turn off to link the system SQLite or SQLCipher |
| `bench` | off | builds the `vecbench` binary (dataset generator and benchmark driver) |
| `simd` | off | NEON (aarch64) and runtime-detected AVX2 + FMA (x86_64) dot products; scalar fallback |
| `parallel` | off | `insert_batch` and `compact` insert on the rayon thread pool |

`insert_batch(&ids, &flat_vectors)` inserts many vectors and saves them in one
transaction. `search_filtered(&q, k, |id| allowed.contains(&id))` searches
with a predicate or allow-list applied inside the graph walk.
`delete(&ids)` tombstones vectors (hidden from search at once, persisted by
`save`); `compact()` rebuilds the graph without them.

Performance features, how to turn each on or off, and why some candidates
were skipped: [docs/features.md](docs/features.md).

The index lives in its own tables (`vdb_meta`, `vdb_page`, `vdb_tomb`); deleting the file
removes it. The qdrant-edge comparison is a separate, unpublished crate in
`bench-qdrant/` (see [docs/benchmark.md](docs/benchmark.md)).

## Choosing parameters

From the recorded benchmark (100k x 384, synthetic): recall@10 >= 0.95 needs
`m = 32` and `ef_search = 1024`. With `Quant::I8` that gave recall 0.95,
p50 10 ms, 58 MiB on disk, ~28 ms cold start, ~3k QPS at 16 threads.
Clustered (embedding-like) data reaches recall 0.986 already at `ef_search = 64`
(~1 ms). Full tables: [docs/benchmark.md](docs/benchmark.md).
`Params::for_recall(Recall::P90 | Recall::P95, quant)` returns these settings
(`m = 32`, `ef_construction = 400`, `ef_search` 768-1280 by mode).

`Quant::I8Rerank` walks the graph with `i8` like `Quant::I8` and rescores the
best `k * rerank` candidates (`Params::rerank`, default 4) with a kept `f32`
copy, so scores are exact `f32` cosine. It costs about 1.25x the memory of
`F32`. See [docs/features.md](docs/features.md).

## Test

```sh
cargo test -p sqlite-hnsw
```

Unit tests sit in `src/lib.rs`; integration tests of the public API in `tests/`.
Benchmarks are manual and never run in CI.

## Limitations

- No in-place update: an id inserted twice is two nodes; update is `delete`
  then `insert`. `delete` scans all nodes, and tombstones cost memory and
  recall until `compact` (a full rebuild).
- Cosine similarity only; vectors are normalized on insert and query.
- `m`, `ef_construction` and quantization are fixed when the file is created;
  reopening keeps the stored values (`ef_search` can change at any time).
- `insert` does not persist: call `save`. Unsaved inserts are lost on crash.
- The whole index is loaded into memory on `open`.
- Incremental insert is slow next to qdrant-edge (about 1-3k vectors/s at
  `m = 32`); fine for steady writes, slow for bulk re-indexing.
- One writer process per file: the SQLite file is not a shared live index.

## License

GPL-3.0-or-later.
