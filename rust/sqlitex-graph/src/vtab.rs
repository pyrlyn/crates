//! Table-valued functions. Traversal runs in Rust over the shared lock-free graph;
//! SQLite only sees the result rows, produced one BFS level at a time while it steps.
//!
//! - `graph_impact(name, depth, root)`        -> depth, path, scope
//! - `graph_impact_many(names, depth, root)`  -> depth, path, scope   (names: one per line)
//! - `graph_neighbors(name, depth, dir, root)` -> depth, name   (dir: 'in' | 'out')
//! - `graph_callers(name, root)`              -> path, scope
//! - `graph_callees(name, root)`              -> path, callee
//!
//! Scalars: `graph_stats()`, `graph_merge(root)`, `graph_reload()`.

use crate::ffi::{Cell, Connection, Error, FunctionFlags, RowStream, TableFunction, Value};
use crate::graph::{Dict, Graph, ImpactRow, ImpactWalk, NeighborsWalk};
use crate::store::{self, Result};
use std::ffi::{CStr, c_int};
use std::path::PathBuf;
use std::sync::Arc;

pub struct Spec {
    name: &'static CStr,
    schema: &'static CStr,
    ncols: c_int,
    nargs: c_int,
    /// Leading result columns the rows are sorted by.
    order: c_int,
    run: fn(&Graph, &[Value]) -> Box<dyn RowStream>,
}

#[derive(Clone)]
enum GraphRef {
    Shared(Arc<Graph>),
    Lazy(PathBuf, Arc<store::GraphSlot>),
}

impl GraphRef {
    fn get(&self) -> Result<Arc<Graph>> {
        match self {
            GraphRef::Shared(g) => Ok(g.clone()),
            GraphRef::Lazy(p, slot) => store::graph_in_slot(slot, p, false),
        }
    }
}

fn text(v: &[Value], i: usize) -> String {
    match v.get(i) {
        Some(Value::Text(s)) => s.clone(),
        Some(Value::Integer(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn int(v: &[Value], i: usize, d: i64) -> i64 {
    match v.get(i) {
        Some(Value::Integer(n)) => *n,
        Some(Value::Text(s)) => s.parse().unwrap_or(d),
        _ => d,
    }
}
/// The depth argument at `i`: default 1, clamped to 0..=64.
fn depth(v: &[Value], i: usize) -> u32 {
    int(v, i, 1).clamp(0, 64) as u32
}

/// A source of sorted row batches (one BFS level, or one whole list) whose ids resolve to
/// text only when SQLite reads the column.
trait Batches: Send + 'static {
    type Row: Send;
    fn next_batch(&mut self) -> Option<Vec<Self::Row>>;
    fn cell<'a>(&'a self, row: &'a Self::Row, col: usize) -> Cell<'a>;
}

struct Batched<B: Batches> {
    src: B,
    rows: Vec<B::Row>,
    pos: usize,
    started: bool,
}

impl<B: Batches> RowStream for Batched<B> {
    fn advance(&mut self) -> bool {
        if self.started {
            self.pos += 1;
        }
        self.started = true;
        while self.pos >= self.rows.len() {
            match self.src.next_batch() {
                Some(b) => {
                    self.rows = b;
                    self.pos = 0;
                }
                None => {
                    self.rows = Vec::new();
                    self.pos = 0;
                    return false;
                }
            }
        }
        true
    }
    fn cell(&self, col: usize) -> Cell<'_> {
        self.rows
            .get(self.pos)
            .map_or(Cell::Null, |r| self.src.cell(r, col))
    }
}

fn stream<B: Batches>(src: Option<B>) -> Box<dyn RowStream> {
    match src {
        Some(src) => Box::new(Batched {
            src,
            rows: Vec::new(),
            pos: 0,
            started: false,
        }),
        None => stream(Some(Pairs::empty())),
    }
}

impl Batches for ImpactWalk {
    type Row = ImpactRow;
    fn next_batch(&mut self) -> Option<Vec<ImpactRow>> {
        self.next_level()
    }
    fn cell<'a>(&'a self, r: &'a ImpactRow, col: usize) -> Cell<'a> {
        match col {
            0 => Cell::Int(r.depth as i64),
            1 => Cell::Text(self.dict().path(r.file)),
            2 => Cell::Text(self.dict().name(r.scope)),
            _ => Cell::Null,
        }
    }
}

impl Batches for NeighborsWalk {
    type Row = (u32, u32);
    fn next_batch(&mut self) -> Option<Vec<(u32, u32)>> {
        self.next_level()
    }
    fn cell<'a>(&'a self, r: &'a (u32, u32), col: usize) -> Cell<'a> {
        match col {
            0 => Cell::Int(r.0 as i64),
            1 => Cell::Text(self.dict().name(r.1)),
            _ => Cell::Null,
        }
    }
}

/// `(file, name)` rows computed in one go (callers, callees).
struct Pairs {
    dict: Arc<Dict>,
    rows: Option<Vec<(u32, u32)>>,
}

impl Pairs {
    fn empty() -> Pairs {
        Pairs {
            dict: Arc::default(),
            rows: None,
        }
    }
}

impl Batches for Pairs {
    type Row = (u32, u32);
    fn next_batch(&mut self) -> Option<Vec<(u32, u32)>> {
        self.rows.take()
    }
    fn cell<'a>(&'a self, r: &'a (u32, u32), col: usize) -> Cell<'a> {
        match col {
            0 => Cell::Text(self.dict.path(r.0)),
            1 => Cell::Text(self.dict.name(r.1)),
            _ => Cell::Null,
        }
    }
}

fn run_impact(g: &Graph, a: &[Value]) -> Box<dyn RowStream> {
    let name = text(a, 0);
    stream(
        g.root_id(&text(a, 2))
            .and_then(|root| g.impact_walk(root, &[name.as_str()], depth(a, 1))),
    )
}
fn run_impact_many(g: &Graph, a: &[Value]) -> Box<dyn RowStream> {
    let names = text(a, 0);
    let names: Vec<&str> = names
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .collect();
    stream(
        g.root_id(&text(a, 2))
            .and_then(|root| g.impact_walk(root, &names, depth(a, 1))),
    )
}
fn run_neighbors(g: &Graph, a: &[Value]) -> Box<dyn RowStream> {
    let out = text(a, 2).eq_ignore_ascii_case("out");
    stream(
        g.root_id(&text(a, 3))
            .and_then(|root| g.neighbors_walk(root, &text(a, 0), depth(a, 1), out)),
    )
}
fn pairs(
    g: &Graph,
    root: usize,
    a: &[Value],
    f: fn(&Graph, u32, &str) -> Vec<(u32, u32)>,
) -> Box<dyn RowStream> {
    stream(g.root_id(&text(a, root)).map(|r| {
        let rows = f(g, r, &text(a, 0));
        // Loaded after the rows: the dictionary only grows, so it knows all their ids.
        Pairs {
            dict: g.dict.load_full(),
            rows: Some(rows),
        }
    }))
}
fn run_callers(g: &Graph, a: &[Value]) -> Box<dyn RowStream> {
    pairs(g, 1, a, Graph::callers)
}
fn run_callees(g: &Graph, a: &[Value]) -> Box<dyn RowStream> {
    pairs(g, 1, a, Graph::callees)
}

pub static SPECS: [Spec; 5] = [
    Spec { name: c"graph_impact", schema: c"CREATE TABLE x(depth INTEGER, path TEXT, scope TEXT, name HIDDEN, max_depth HIDDEN, root HIDDEN)", ncols: 3, nargs: 3, order: 3, run: run_impact },
    Spec { name: c"graph_impact_many", schema: c"CREATE TABLE x(depth INTEGER, path TEXT, scope TEXT, names HIDDEN, max_depth HIDDEN, root HIDDEN)", ncols: 3, nargs: 3, order: 3, run: run_impact_many },
    Spec { name: c"graph_neighbors", schema: c"CREATE TABLE x(depth INTEGER, name TEXT, start HIDDEN, max_depth HIDDEN, dir HIDDEN, root HIDDEN)", ncols: 2, nargs: 4, order: 2, run: run_neighbors },
    Spec { name: c"graph_callers", schema: c"CREATE TABLE x(path TEXT, scope TEXT, name HIDDEN, root HIDDEN)", ncols: 2, nargs: 2, order: 2, run: run_callers },
    Spec { name: c"graph_callees", schema: c"CREATE TABLE x(path TEXT, callee TEXT, name HIDDEN, root HIDDEN)", ncols: 2, nargs: 2, order: 2, run: run_callees },
];

struct Tvf {
    spec: &'static Spec,
    graph: GraphRef,
}

impl TableFunction for Tvf {
    fn schema(&self) -> &'static CStr {
        self.spec.schema
    }
    fn ncols(&self) -> c_int {
        self.spec.ncols
    }
    fn nargs(&self) -> c_int {
        self.spec.nargs
    }
    fn order_cols(&self) -> c_int {
        self.spec.order
    }
    fn open(&self, args: &[Value]) -> Result<Box<dyn RowStream>> {
        let g = self.graph.get()?;
        Ok((self.spec.run)(&g, args))
    }
}

fn register_impl(conn: &Connection, graph: GraphRef) -> Result<()> {
    for spec in SPECS.iter() {
        conn.create_table_function(
            spec.name,
            Box::new(Tvf {
                spec,
                graph: graph.clone(),
            }),
        )?;
    }
    let g1 = graph.clone();
    conn.create_scalar_function("graph_stats", 0, FunctionFlags::SQLITE_UTF8, move |_| {
        Ok(g1.get().map_err(|e| Error::User(e.to_string()))?.stats())
    })?;
    let path = match &graph {
        GraphRef::Lazy(p, _) => Some(p.clone()),
        GraphRef::Shared(_) => conn.path().filter(|p| !p.is_empty()).map(PathBuf::from),
    };
    let (g2, p2) = (graph.clone(), path.clone());
    conn.create_scalar_function("graph_merge", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let root: String = ctx.get(0)?;
        let g = g2.get().map_err(|e| Error::User(e.to_string()))?;
        let Some(rid) = g.root_id(&root) else {
            return Ok(0i64);
        };
        let p = p2
            .clone()
            .ok_or_else(|| Error::User("in-memory database".into()))?;
        store::merge_root(&g, &p, rid).map_err(|e| Error::User(e.to_string()))?;
        Ok(1i64)
    })?;
    conn.create_scalar_function("graph_reload", 0, FunctionFlags::SQLITE_UTF8, move |_| {
        let p = path
            .clone()
            .ok_or_else(|| Error::User("in-memory database".into()))?;
        store::graph_for(&p, true).map_err(|e| Error::User(e.to_string()))?;
        Ok(1i64)
    })?;
    Ok(())
}

/// Register on a connection; the graph is loaded lazily (once per process per DB file).
pub fn register(conn: &Connection) -> Result<()> {
    let p = conn
        .path()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Error::Module("sqlitex-graph needs a file database".into()))?;
    let p = PathBuf::from(p);
    let slot = store::slot_for(&p);
    register_impl(conn, GraphRef::Lazy(p, slot))
}

/// Register using an already-loaded graph (rlib use with [`crate::Store`]).
pub fn register_with(conn: &Connection, graph: Arc<Graph>) -> Result<()> {
    register_impl(conn, GraphRef::Shared(graph))
}

impl std::fmt::Debug for Spec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spec").finish_non_exhaustive()
    }
}
