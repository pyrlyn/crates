# How sqlite-hnsw works

## Graph

HNSW (Malkov & Yashunin). Each node gets a random top level; upper layers keep
`m` neighbours, layer 0 keeps `2 * m`. Insert descends greedily to the node's
level, then runs a beam search of width `ef_construction` per layer and keeps
neighbours chosen by the diversity heuristic; reverse links are added and
trimmed by the same heuristic when a list overflows. Search descends greedily
and runs one beam of width `max(ef_search, k)` on layer 0.

## Vectors

Distance is `1 - dot` on normalized vectors (cosine); `search` returns
`1 - distance`, the cosine similarity. With `Quant::I8` each vector is stored
as `i8` components plus one `f32` scale (`max |x| / 127`); the query stays
`f32`, so the dot product is `f32 x i8` with one multiply by the scale.

## Concurrency

Nodes live in an append-only arena of doubling buckets, so reading a node
never locks. Every adjacency list has its own `parking_lot::RwLock`; an insert
write-locks only the lists it rewrites and the entry point only when the new
node is higher. `insert` and `search` can run from any number of threads.

## Persistence

One SQLite file in WAL mode with two tables: `vdb_meta` (dimension, `m`,
`ef_construction`, quantization, entry point) and `vdb_page` (1024 nodes per
BLOB). Inserts mark pages dirty; `save` rewrites only dirty pages in one
transaction. `open` reads the pages through SQLite's mmap I/O and parses them
straight into the arena. The SQLite binding is a small FFI layer over
`libsqlite3-sys` (`src/sqlite.rs`, the only module with `unsafe`).
