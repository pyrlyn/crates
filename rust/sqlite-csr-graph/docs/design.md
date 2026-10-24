# How sqlite-csr-graph works

**SQLite stays the source of truth** for symbols, in a normalized schema (`src/store.rs`):

| table | contents |
|---|---|
| `roots`, `files` | each root/path stored once; `sha` as a 32-byte BLOB (not 64 hex chars per row); `gen` = write generation |
| `names`, `kinds` | interned strings; symbols reference integer ids |
| `defs` | definitions (signature/doc only here) — covering indexes `(name_id, file_id, ...)`, `(file_id, line, ...)` |
| `refs` | references — covering indexes `refs_name(name_id, file_id, kind, line, scope_id)` and `refs_scope(file_id, scope_id, ...)` (fixes the unindexed `callees` self-join) |
| `defs_fts_<root>` | one FTS5 external-content index **per root** (no more "match all roots, then filter") |
| `graph_segments` | the graph's immutable CSR segments as BLOBs |

`ANALYZE` runs at import, so the planner never picks the wrong index on a fresh DB.

**The graph lives in immutable CSR segments** (`src/segment.rs`) per root:
`users` (name → referencing (file, owner scope) / importing file), `callees` (owner → (file, callee)),
`filedefs` (file → defined names). Layout: sorted `u32` keys + `u32` offsets + LEB128 varint
payload with delta-coded file ids and the edge kind packed into the low bit. Readers decode
directly from the blob (≈3.8 bytes per edge on the rtok index).

**Writes** (`Store::upsert_file` / `delete_file`) go to SQLite in one transaction, then publish a
`FileOp` into a small in-memory **delta** (per-file replacement + tombstone bitmap of superseded
files). When the delta grows past `merge_threshold`, a **background merge** folds base + delta into
a new segment, persists it, and swaps it in with an `ArcSwap` pointer store. Ops that arrived during
the merge are kept. On open, files with `gen` newer than the persisted segments are replayed into
the delta, so nothing is lost if the process dies before a merge.

**Concurrency**: readers load an `Arc<Snapshot>` (base + delta) from `ArcSwap` — no locks, no
blocking on writers or merges. There is exactly one writer at a time (`Mutex` on the write path
only; SQLite WAL also has a single writer). SQLite reads use one connection per thread.

**Native traversal** (`src/graph.rs`, `src/vtab.rs`) runs in Rust over the segments and is exposed
as eponymous table-valued functions:

```sql
SELECT depth, path, scope FROM graph_impact('Store', 2, '/path/to/root');
SELECT depth, name        FROM graph_neighbors('Store', 2, 'in' /* or 'out' */, '/path/to/root');
SELECT path, scope        FROM graph_callers('Store', '/path/to/root');
SELECT path, callee       FROM graph_callees('Store', '/path/to/root');
SELECT graph_stats();          -- JSON: segment sizes, edge counts, delta size
SELECT graph_merge('/path/to/root');   -- fold the delta now
SELECT graph_reload();         -- re-read the graph (after another process wrote)
```

`graph_impact` uses level-by-level BFS with a **global visited set** (frontier dedup) instead of
rtok's per-path visited sets, so its cost is bounded by the number of reachable edges.

## Impact semantics

rtok skips an edge whose owner already appears on the *current path* and enumerates every path.
`graph_impact` visits each owner once. Verified on the rtok index (`tests/equivalence.rs`, 713 queries):
every (path, scope) row rtok returns is returned, at the same or a smaller depth; the set of
reachable symbols is identical; the only extra rows are cycle-closing edges to an owner already
reached at a smaller depth (4,877 such rows across the sample). `defs`, `refs`, `callers`
(ref groups), `callees`, `outline` and FTS match rtok row-for-row.

## rtok import

`import_rtok` (feature `rtok-import`) converts rtok's denormalized `symbols` + `file_rank`
tables into this schema, then builds the FTS indexes and the segments and runs `ANALYZE`.
It is the only code that knows rtok's schema.
