# sqlite-csr-graph

A code-symbol store in SQLite with a native reference/call graph. Symbols live in
ordinary SQLite tables; the graph (who uses a name, what a function calls, what a
file defines) lives in compact immutable CSR segments and is traversed in Rust,
exposed to SQL as table-valued functions:

```sql
SELECT depth, path, scope FROM graph_impact('Store', 3, '/path/to/root');
SELECT depth, name        FROM graph_neighbors('Store', 2, 'in', '/path/to/root'); -- or 'out'
SELECT path, scope        FROM graph_callers('Store', '/path/to/root');
SELECT path, callee       FROM graph_callees('Store', '/path/to/root');
SELECT graph_stats();               -- JSON per root: segment bytes, edge counts, delta size
SELECT graph_merge('/path/to/root'); -- fold the in-memory delta into a new segment now
SELECT graph_reload();              -- re-read after another process wrote
```

From Rust:

```rust,no_run
use sqlite_csr_graph::{DefRow, FileSymbols, RefRow, Store};

let store = Store::open("index.db".as_ref())?;            // the one writer
store.upsert_file(&FileSymbols {
    root: "/repo".into(),
    path: "src/main.rs".into(),
    defs: vec![DefRow { name: "main".into(), kind: "function".into(), ..Default::default() }],
    refs: vec![RefRow { name: "run".into(), scope: "main".into(), kind: "call".into(), ..Default::default() }],
    ..Default::default()
})?;
let conn = store.reader()?;                                // one per reader thread, lock-free graph
let n: i64 = conn.query_row("SELECT count(*) FROM graph_callers('run', '/repo')", [], |r| r.get(0))?;
# Ok::<(), rusqlite::Error>(())
```

- Readers never block: they load an `Arc` snapshot (base segments + delta).
- Writes replace a file's symbols in one SQLite transaction and publish a small delta;
  a background merge folds it into new segments.
- ≈3.8 bytes per edge; on a 317k-symbol index the database is 3.4× smaller than the
  denormalized one it replaced, and `impact` at depth 3 on hot names takes ~14 ms
  instead of ~11 s. Full numbers: [docs/benchmarks.md](docs/benchmarks.md).

## Enable

| Use | Build |
| --- | --- |
| Rust library, bundled SQLite (default) | `sqlite-csr-graph = "0.1"` |
| Rust library, your own SQLite / SQLCipher | `default-features = false`, enable a `libsqlite3-sys` link feature yourself |
| SQLite loadable extension | `cargo build --release --no-default-features --features loadable --lib` |
| rtok index importer and benchmarks | `--features rtok-import` |

Load the extension: `.load target/release/libsqlite_csr_graph sqlite3_extension_init` in the
`sqlite3` shell, `conn.load_extension(path, entrypoint="sqlite3_extension_init")` in Python,
or `sqlite_csr_graph::register(&conn)` on any rusqlite connection to a file database.

Multi-threaded hosts should call `sqlite_csr_graph::disable_sqlite_memstatus()` before the
first connection (not in the loadable build: the host owns SQLite). SQLite's default memory
statistics take a global mutex on every allocation; turning them off took 16-thread reads
from 19k to 276k queries/s.

## Disable

Don't load the extension or don't call `register`: the tables are plain SQLite and stay
queryable with ordinary SQL. Dropping `graph_segments` is safe too; the graph is rebuilt
from `refs`/`defs` on the next load.

## Test

```sh
cargo test -p sqlite-csr-graph                 # unit, SQL functions on a fixture, concurrency + merge
RTOK_OLD_DB=/path/to/rtok.db cargo test -p sqlite-csr-graph --release \
  --features rtok-import --test equivalence -- --ignored   # row-for-row against a real rtok index
```

## Limitations

- File databases only (the graph is shared per database path within a process).
- Writes from another process are visible after `graph_reload()` or reopening.
- Writes must go through `Store`; raw inserts into `refs`/`defs` bypass the delta until
  `graph_reload()`.
- One writer at a time (SQLite WAL). Parallel writes to different roots would need one file
  per root; not implemented.
- `graph_impact` visits each owner once (global visited set). Compared with a per-path
  traversal it returns the same reachable set but may add cycle-closing rows; see
  [docs/design.md](docs/design.md#impact-semantics).
- Full-text search is plain FTS5 and is ~10–15 % slower than a single global FTS index on the
  benchmark (per-root external-content index).

More: [docs/design.md](docs/design.md) (how it works), [docs/benchmarks.md](docs/benchmarks.md).

Licensed under either of MIT or Apache-2.0 at your option.
