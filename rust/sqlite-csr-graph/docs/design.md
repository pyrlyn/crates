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
directly from the blob (≈3.8 bytes per edge on a real 317k-symbol code index).

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
per-path visited sets, so its cost is bounded by the number of reachable edges.

## Impact semantics

A per-path traversal skips an edge whose owner already appears on the *current path* and
enumerates every path. `graph_impact` visits each owner once: the set of reachable symbols is the
same, and every (path, scope) row a per-path traversal returns is returned at the same or a
smaller depth; the only extra rows are cycle-closing edges to an owner already reached at a
smaller depth.

## Why libsqlite3-sys, not rusqlite

Typical hosts talk to SQLite through diesel, which links `libsqlite3-sys`. Only one
crate in a build may link `sqlite3`, so depending on rusqlite forced its `libsqlite3-sys` line on
every host and pulled a second wrapper into diesel programs. The crate now depends on
`libsqlite3-sys` 0.38 (the workspace's version) and carries its own minimal wrapper in
`src/ffi.rs`: connections, prepared and cached statements, owned `Value`s, scalar functions,
and the virtual-table module.

The table-valued functions are one `sqlite3_module` registered per name with
`sqlite3_create_module_v2`. `xCreate` is null, so they are eponymous-only. `xBestIndex` maps
`=` constraints on the hidden argument columns to `argv` slots (bitmask in `idxNum`) and
refuses a plan where such a constraint is unusable; `xFilter` runs the traversal in Rust and
materializes the rows; `xNext`/`xEof`/`xColumn`/`xRowid` walk them.

Three ways in:

- `register(&conn)` / `register_with(&conn, graph)` on a wrapper connection (`Store::reader`).
- `sqlite3_extension_init` in the loadable build (`loadable` feature, which turns on
  `libsqlite3-sys/loadable_extension` so every SQLite call goes through the host's
  `sqlite3_api_routines`).
- `register_auto_extension()` (`sqlite3_auto_extension`): every later connection in the
  process, e.g. diesel's, gets the functions.

## Unsafe policy

`unsafe_code = "deny"` for the crate; `src/ffi.rs` alone allows it, and each `unsafe` block or
function says why it is sound. Callbacks from SQLite (`x*`, scalar functions, destructors,
entry points) run inside `catch_unwind`; a panic becomes `SQLITE_ERROR` or a function error.
Boxes handed to SQLite (`pClientData`, `pApp`, vtabs, cursors) are freed only in the matching
SQLite destructor.
