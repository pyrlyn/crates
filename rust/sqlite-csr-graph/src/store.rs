//! SQLite side: normalized schema (source of truth),
//! graph load/persist, and the single-writer update API.

use crate::ffi::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use crate::graph::{Base, Delta, Dict, FileOp, Graph, Snapshot};
use crate::params;
use crate::segment::{Csr, Edge, KIND_IMPORT, KIND_REF};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Result of every store operation.
pub type Result<T> = crate::ffi::Result<T>;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS roots(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS files(
  id INTEGER PRIMARY KEY, root_id INTEGER NOT NULL, path TEXT NOT NULL,
  sha BLOB NOT NULL, mtime INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0,
  gen INTEGER NOT NULL DEFAULT 0, UNIQUE(root_id, path));
CREATE INDEX IF NOT EXISTS files_gen ON files(gen);
CREATE TABLE IF NOT EXISTS names(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS kinds(id INTEGER PRIMARY KEY, kind TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS defs(
  id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, name_id INTEGER NOT NULL, kind INTEGER NOT NULL,
  line INTEGER NOT NULL, end_line INTEGER NOT NULL, start_byte INTEGER NOT NULL, end_byte INTEGER NOT NULL,
  content_hash BLOB, signature TEXT, doc TEXT);
CREATE INDEX IF NOT EXISTS defs_name ON defs(name_id, file_id, line, kind, end_line);
CREATE INDEX IF NOT EXISTS defs_file ON defs(file_id, line, name_id, kind, end_line);
CREATE TABLE IF NOT EXISTS refs(
  id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, name_id INTEGER NOT NULL, scope_id INTEGER NOT NULL,
  kind INTEGER NOT NULL, line INTEGER NOT NULL, start_byte INTEGER NOT NULL, end_byte INTEGER NOT NULL,
  content_hash BLOB);
CREATE INDEX IF NOT EXISTS refs_name ON refs(name_id, file_id, kind, line, scope_id);
CREATE INDEX IF NOT EXISTS refs_scope ON refs(file_id, scope_id, name_id, line);
CREATE TABLE IF NOT EXISTS file_rank(root_id INTEGER PRIMARY KEY, graph TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS graph_segments(root_id INTEGER NOT NULL, part TEXT NOT NULL, gen INTEGER NOT NULL,
  data BLOB NOT NULL, PRIMARY KEY(root_id, part)) WITHOUT ROWID;
"#;

/// Per-root FTS5 index (external content over a per-root view): a search touches only
/// its own root's postings, unlike a single global index filtered after matching.
pub fn ensure_root_fts(conn: &Connection, root: u32) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIEW IF NOT EXISTS fts_src_{root} AS SELECT d.id AS id, n.name AS name, d.signature AS signature, d.doc AS doc \
         FROM defs d JOIN names n ON n.id=d.name_id JOIN files f ON f.id=d.file_id WHERE d.name_id<>0 AND f.root_id={root};
         CREATE VIRTUAL TABLE IF NOT EXISTS defs_fts_{root} USING fts5(name, signature, doc, content='fts_src_{root}', content_rowid='id');"
    ))
}

pub fn fts_table(root: i64) -> String {
    format!("defs_fts_{root}")
}

/// Reference query shapes over the normalized schema (used by tests). `?1` root id, `?2` name/path.
pub mod sql {
    pub const DEFS: &str = "SELECT f.path, k.kind, d.line, d.end_line FROM defs d JOIN files f ON f.id=d.file_id JOIN kinds k ON k.id=d.kind \
        WHERE d.name_id=(SELECT id FROM names WHERE name=?2) AND f.root_id=?1 ORDER BY f.path, d.line";
    pub const REFS: &str = "SELECT f.path, r.line FROM refs r JOIN files f ON f.id=r.file_id \
        WHERE r.name_id=(SELECT id FROM names WHERE name=?2) AND f.root_id=?1 AND r.kind<>(SELECT id FROM kinds WHERE kind='import') ORDER BY f.path, r.line";
    pub const REF_GROUPS: &str = "SELECT f.path, s.name, count(*), min(r.line) FROM refs r JOIN files f ON f.id=r.file_id JOIN names s ON s.id=r.scope_id \
        WHERE r.name_id=(SELECT id FROM names WHERE name=?2) AND f.root_id=?1 AND r.kind<>(SELECT id FROM kinds WHERE kind='import') \
        GROUP BY f.path, s.name ORDER BY f.path, s.name";
    pub const CALLEES: &str = "SELECT f.path, d.line, n.name, r.line FROM defs d JOIN files f ON f.id=d.file_id \
        JOIN refs r ON r.file_id=d.file_id AND r.scope_id=d.name_id JOIN names n ON n.id=r.name_id \
        WHERE d.name_id=(SELECT id FROM names WHERE name=?2) AND f.root_id=?1 AND r.name_id<>0";
    pub const FILE_DEFS: &str = "SELECT n.name, k.kind, d.line, d.end_line FROM defs d JOIN names n ON n.id=d.name_id JOIN kinds k ON k.id=d.kind \
        WHERE d.file_id=(SELECT id FROM files WHERE root_id=?1 AND path=?2) ORDER BY d.line";
    /// FTS over one root; `{t}` = [`super::fts_table`], `?1` = query.
    pub fn fts(table: &str) -> String {
        format!(
            "SELECT f.path, n.name, k.kind, d.line, d.signature, d.doc, bm25({table}) FROM {table} \
            JOIN defs d ON d.id={table}.rowid JOIN files f ON f.id=d.file_id JOIN names n ON n.id=d.name_id JOIN kinds k ON k.id=d.kind \
            WHERE {table} MATCH ?1 LIMIT 200"
        )
    }
}

pub fn tune(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-65536; \
         PRAGMA mmap_size=268435456; PRAGMA temp_store=MEMORY; PRAGMA busy_timeout=10000;",
    )
}

pub(crate) fn import_kind(conn: &Connection) -> Result<i64> {
    Ok(conn
        .query_row("SELECT id FROM kinds WHERE kind='import'", [], |r| r.get(0))
        .optional()?
        .unwrap_or(-1))
}

/// Build the immutable base for a root straight from the symbol tables.
pub fn build_base(conn: &Connection, root: u32) -> Result<Base> {
    let imp = import_kind(conn)?;
    let mut users = Vec::new();
    let mut callees = Vec::new();
    let mut st = conn.prepare(
        "SELECT r.name_id, r.file_id, r.scope_id, r.kind FROM refs r JOIN files f ON f.id=r.file_id WHERE f.root_id=?1",
    )?;
    let mut rows = st.query([root])?;
    while let Some(r) = rows.next()? {
        let (name, file, scope, kind): (u32, u32, u32, i64) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        push_ref_edges(&mut users, &mut callees, name, file, scope, kind == imp);
    }
    let mut fdefs = Vec::new();
    let mut st = conn.prepare("SELECT d.file_id, d.name_id FROM defs d JOIN files f ON f.id=d.file_id WHERE f.root_id=?1 AND d.name_id<>0")?;
    let mut rows = st.query([root])?;
    while let Some(r) = rows.next()? {
        fdefs.push((r.get(0)?, r.get(1)?));
    }
    let generation: i64 = conn.query_row(
        "SELECT coalesce(max(gen),0) FROM files WHERE root_id=?1",
        [root],
        |r| r.get(0),
    )?;
    Ok(Base::from_pairs(users, callees, fdefs, generation))
}

/// Edge derivation for one ref row (the impact/callees predicates).
#[inline]
fn push_ref_edges(
    users: &mut Vec<(u32, Edge)>,
    callees: &mut Vec<(u32, Edge)>,
    name: u32,
    file: u32,
    scope: u32,
    is_import: bool,
) {
    if name == 0 {
        return;
    }
    if is_import {
        users.push((
            name,
            Edge {
                file,
                kind: KIND_IMPORT,
                other: 0,
            },
        ));
    } else {
        users.push((
            name,
            Edge {
                file,
                kind: KIND_REF,
                other: scope,
            },
        ));
    }
    if scope != 0 {
        callees.push((
            scope,
            Edge {
                file,
                kind: KIND_REF,
                other: name,
            },
        ));
    }
}

pub fn persist_base(conn: &Connection, root: u32, base: &Base) -> Result<()> {
    let mut st = conn.prepare(
        "INSERT OR REPLACE INTO graph_segments(root_id, part, gen, data) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (part, csr) in [
        ("users", &base.users),
        ("callees", &base.callees),
        ("filedefs", &base.filedefs),
    ] {
        st.execute(params![root, part, base.generation, csr.bytes()])?;
    }
    Ok(())
}

fn load_dict(conn: &Connection) -> Result<Dict> {
    let mut d = Dict::default();
    for r in conn
        .prepare("SELECT id, name FROM names")?
        .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))?
    {
        let (id, n) = r?;
        d.set_name(id, &n);
    }
    for r in conn
        .prepare("SELECT id, root_id, path FROM files")?
        .query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
    {
        let (id, root, p) = r?;
        d.set_file(id, root, &p);
    }
    for r in conn
        .prepare("SELECT id, path FROM roots")?
        .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))?
    {
        let (id, p) = r?;
        d.roots.insert(p, id);
    }
    Ok(d)
}

fn file_op(conn: &Connection, file: u32, generation: i64, imp: i64) -> Result<FileOp> {
    let mut op = FileOp {
        file,
        generation,
        ..Default::default()
    };
    let mut st =
        conn.prepare_cached("SELECT name_id, scope_id, kind FROM refs WHERE file_id=?1")?;
    let mut rows = st.query([file])?;
    while let Some(r) = rows.next()? {
        push_ref_edges(
            &mut op.users,
            &mut op.callees,
            r.get(0)?,
            file,
            r.get(1)?,
            r.get::<_, i64>(2)? == imp,
        );
    }
    let mut st = conn.prepare_cached(
        "SELECT DISTINCT name_id FROM defs WHERE file_id=?1 AND name_id<>0 ORDER BY 1",
    )?;
    op.defs = st.query_map([file], |r| r.get(0))?.collect::<Result<_>>()?;
    op.users.sort_unstable();
    op.users.dedup();
    op.callees.sort_unstable();
    op.callees.dedup();
    Ok(op)
}

/// Load the graph for a database: persisted segments, rebuilt if missing, plus a delta
/// replayed from files newer than the segments.
pub fn load_graph(conn: &Connection) -> Result<Graph> {
    let dict = load_dict(conn)?;
    let imp = import_kind(conn)?;
    let mut roots = HashMap::new();
    for &rid in dict.roots.values() {
        let mut parts: HashMap<String, (i64, Vec<u8>)> = HashMap::new();
        for r in conn
            .prepare("SELECT part, gen, data FROM graph_segments WHERE root_id=?1")?
            .query_map([rid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })?
        {
            let (p, g, d) = r?;
            parts.insert(p, (g, d));
        }
        let seg = |p: &str| {
            parts
                .get(p)
                .and_then(|(_, d)| Csr::from_blob(d.clone()).ok())
        };
        let base = match (seg("users"), seg("callees"), seg("filedefs")) {
            (Some(users), Some(callees), Some(filedefs)) => Base {
                users,
                callees,
                filedefs,
                generation: parts["users"].0,
            },
            _ => build_base(conn, rid)?,
        };
        let mut ops = Vec::new();
        let files: Vec<(u32, i64)> = conn
            .prepare("SELECT id, gen FROM files WHERE root_id=?1 AND gen>?2 ORDER BY gen")?
            .query_map(params![rid, base.generation], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_>>()?;
        for (i, (f, g)) in files.into_iter().enumerate() {
            let mut op = file_op(conn, f, g, imp)?;
            op.seq = i as u64;
            ops.push(Arc::new(op));
        }
        roots.insert(
            rid,
            Snapshot {
                base: Arc::new(base),
                delta: Arc::new(Delta::from_ops(ops)),
            },
        );
    }
    Ok(Graph::new(dict, roots))
}

/// Process-wide registry so every connection to the same file shares one graph.
/// The registry mutex is only taken when a connection registers; queries go through the
/// per-file `ArcSwapOption` slot (lock-free).
pub type GraphSlot = arc_swap::ArcSwapOption<Graph>;

fn registry() -> &'static Mutex<HashMap<PathBuf, Arc<GraphSlot>>> {
    static R: OnceLock<Mutex<HashMap<PathBuf, Arc<GraphSlot>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn slot_for(path: &Path) -> Arc<GraphSlot> {
    registry()
        .lock()
        .unwrap()
        .entry(canon(path))
        .or_default()
        .clone()
}

/// Load (or reuse) the graph for `path`; `reload` forces a re-read from the database.
pub fn graph_in_slot(slot: &GraphSlot, path: &Path, reload: bool) -> Result<Arc<Graph>> {
    if !reload && let Some(g) = slot.load_full() {
        return Ok(g);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let g = Arc::new(load_graph(&conn)?);
    slot.store(Some(g.clone()));
    Ok(g)
}

pub fn graph_for(path: &Path, reload: bool) -> Result<Arc<Graph>> {
    graph_in_slot(&slot_for(path), path, reload)
}

// ---------------------------------------------------------------- writer API

#[derive(Debug, Clone, Default)]
pub struct DefRow {
    pub name: String,
    pub kind: String,
    pub line: i64,
    pub end_line: i64,
    pub start_byte: i64,
    pub end_byte: i64,
    pub content_hash: Option<Vec<u8>>,
    pub signature: Option<String>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RefRow {
    pub name: String,
    pub scope: String,
    pub kind: String,
    pub line: i64,
    pub start_byte: i64,
    pub end_byte: i64,
    pub content_hash: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct FileSymbols {
    pub root: String,
    pub path: String,
    pub sha: Vec<u8>,
    pub mtime: i64,
    pub size: i64,
    pub defs: Vec<DefRow>,
    pub refs: Vec<RefRow>,
}

/// Single writer + shared lock-free graph. Readers use [`Store::reader`] (one connection per thread).
pub struct Store {
    path: PathBuf,
    conn: Mutex<Connection>,
    pub graph: Arc<Graph>,
    merging: Arc<AtomicBool>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        tune(&conn)?;
        conn.execute_batch(SCHEMA)?;
        conn.execute("INSERT OR IGNORE INTO names(id, name) VALUES (0, '')", [])?;
        let graph = graph_for(path, true)?;
        Ok(Store {
            path: path.to_path_buf(),
            conn: Mutex::new(conn),
            graph,
            merging: Arc::new(AtomicBool::new(false)),
        })
    }

    /// A new read connection with the graph table-valued functions registered.
    pub fn reader(&self) -> Result<Connection> {
        let c = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        tune(&c)?;
        crate::vtab::register_with(&c, self.graph.clone())?;
        Ok(c)
    }

    fn intern(
        tx: &Connection,
        table: &str,
        col: &str,
        v: &str,
        dict_new: &mut Vec<(u32, String)>,
    ) -> Result<u32> {
        if let Some(id) = tx
            .prepare_cached(&format!("SELECT id FROM {table} WHERE {col}=?1"))?
            .query_row([v], |r| r.get(0))
            .optional()?
        {
            return Ok(id);
        }
        tx.prepare_cached(&format!("INSERT INTO {table}({col}) VALUES (?1)"))?
            .execute([v])?;
        let id = tx.last_insert_rowid() as u32;
        if table == "names" {
            dict_new.push((id, v.to_string()));
        }
        Ok(id)
    }

    /// Replace all symbols of one file (SQLite first, then publish to the graph).
    pub fn upsert_file(&self, fs: &FileSymbols) -> Result<()> {
        self.write_file(fs, false)
    }
    pub fn delete_file(&self, root: &str, path: &str) -> Result<()> {
        self.write_file(
            &FileSymbols {
                root: root.into(),
                path: path.into(),
                ..Default::default()
            },
            true,
        )
    }

    fn write_file(&self, fs: &FileSymbols, delete: bool) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let _w = self.graph.writer.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut new_names = vec![];
        let generation: i64 = tx
            .query_row(
                "UPDATE meta SET value=value+1 WHERE key='gen' RETURNING value",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| {
                tx.execute("INSERT INTO meta VALUES('gen', 1)", []).ok();
                1
            });
        tx.execute("INSERT OR IGNORE INTO roots(path) VALUES (?1)", [&fs.root])?;
        let root: u32 = tx.query_row("SELECT id FROM roots WHERE path=?1", [&fs.root], |r| {
            r.get(0)
        })?;
        tx.execute(
            "INSERT INTO files(root_id, path, sha, mtime, size, gen) VALUES (?1,?2,?3,?4,?5,?6) \
             ON CONFLICT(root_id, path) DO UPDATE SET sha=excluded.sha, mtime=excluded.mtime, size=excluded.size, gen=excluded.gen",
            params![root, fs.path, fs.sha, fs.mtime, fs.size, generation],
        )?;
        let file: u32 = tx.query_row(
            "SELECT id FROM files WHERE root_id=?1 AND path=?2",
            params![root, fs.path],
            |r| r.get(0),
        )?;
        ensure_root_fts(&tx, root)?;
        let fts = fts_table(root as i64);
        tx.execute(
            &format!("INSERT INTO {fts}({fts}, rowid, name, signature, doc) SELECT 'delete', id, name, signature, doc FROM fts_src_{root} \
             WHERE id IN (SELECT id FROM defs WHERE file_id=?1)"),
            [file],
        )?;
        tx.execute("DELETE FROM defs WHERE file_id=?1", [file])?;
        tx.execute("DELETE FROM refs WHERE file_id=?1", [file])?;
        if !delete {
            for d in &fs.defs {
                let n = Self::intern(&tx, "names", "name", &d.name, &mut new_names)?;
                let k = Self::intern(&tx, "kinds", "kind", &d.kind, &mut new_names)?;
                tx.prepare_cached("INSERT INTO defs(file_id,name_id,kind,line,end_line,start_byte,end_byte,content_hash,signature,doc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?
                    .execute(params![file, n, k, d.line, d.end_line, d.start_byte, d.end_byte, d.content_hash, d.signature, d.doc])?;
                if n != 0 {
                    let id = tx.last_insert_rowid();
                    tx.prepare_cached(&format!(
                        "INSERT INTO {fts}(rowid, name, signature, doc) VALUES (?1,?2,?3,?4)"
                    ))?
                    .execute(params![id, d.name, d.signature, d.doc])?;
                }
            }
            for r in &fs.refs {
                let n = Self::intern(&tx, "names", "name", &r.name, &mut new_names)?;
                let s = Self::intern(&tx, "names", "name", &r.scope, &mut new_names)?;
                let k = Self::intern(&tx, "kinds", "kind", &r.kind, &mut new_names)?;
                tx.prepare_cached("INSERT INTO refs(file_id,name_id,scope_id,kind,line,start_byte,end_byte,content_hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)")?
                    .execute(params![file, n, s, k, r.line, r.start_byte, r.end_byte, r.content_hash])?;
            }
        }
        let imp = import_kind(&tx)?;
        let op = file_op(&tx, file, generation, imp)?;
        tx.commit()?;
        // Publish: dictionary first (so readers can resolve new ids), then the root delta.
        let cur = self.graph.dict.load();
        if !new_names.is_empty()
            || cur
                .files
                .get(file as usize)
                .and_then(|f| f.as_ref())
                .is_none()
            || !cur.roots.contains_key(&fs.root)
        {
            let mut d = (**cur).clone();
            for (id, n) in &new_names {
                d.set_name(*id, n);
            }
            d.set_file(file, root, &fs.path);
            d.roots.insert(fs.root.clone(), root);
            self.graph.dict.store(Arc::new(d));
        }
        let snap = self.graph.apply_locked(root, op);
        drop(_w);
        drop(conn);
        if snap.delta.edges + snap.delta.ops.len() * 16 > self.graph.merge_threshold {
            self.spawn_merge(root);
        }
        Ok(())
    }

    /// Background merge: fold delta into a new base, persist it, swap the pointer.
    pub fn spawn_merge(&self, root: u32) -> Option<std::thread::JoinHandle<Result<()>>> {
        if self.merging.swap(true, Ordering::AcqRel) {
            return None;
        }
        let (g, path, flag) = (self.graph.clone(), self.path.clone(), self.merging.clone());
        Some(std::thread::spawn(move || {
            let r = merge_root(&g, &path, root);
            flag.store(false, Ordering::Release);
            r
        }))
    }

    pub fn merge_now(&self, root: u32) -> Result<()> {
        merge_root(&self.graph, &self.path, root)
    }
}

pub fn merge_root(g: &Graph, path: &Path, root: u32) -> Result<()> {
    let Some(snap) = g.snapshot(root) else {
        return Ok(());
    };
    if snap.delta.ops.is_empty() {
        return Ok(());
    }
    let base = snap.compact();
    let conn = Connection::open(path)?;
    tune(&conn)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    persist_base(&conn, root, &base)?;
    conn.execute_batch("COMMIT")?;
    g.install_merged(root, &snap, base);
    Ok(())
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}
