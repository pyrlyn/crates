//! Table-valued functions. Traversal runs in Rust over the shared lock-free graph;
//! SQLite only sees the result rows.
//!
//! - `graph_impact(name, depth, root)`        -> depth, path, scope
//! - `graph_neighbors(name, depth, dir, root)` -> depth, name   (dir: 'in' | 'out')
//! - `graph_callers(name, root)`              -> path, scope
//! - `graph_callees(name, root)`              -> path, callee
//!
//! Scalars: `graph_stats()`, `graph_merge(root)`, `graph_reload()`.

use crate::ffi::{Connection, Error, FunctionFlags, TableFunction, Value};
use crate::graph::Graph;
use crate::store::{self, Result};
use std::ffi::{CStr, c_int};
use std::path::PathBuf;
use std::sync::Arc;

type Rows = Vec<Vec<Value>>;

pub struct Spec {
    name: &'static CStr,
    schema: &'static CStr,
    ncols: c_int,
    nargs: c_int,
    run: fn(&Graph, &[Value]) -> Rows,
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
fn s(x: &str) -> Value {
    Value::Text(x.to_string())
}

fn run_impact(g: &Graph, a: &[Value]) -> Rows {
    let Some(root) = g.root_id(&text(a, 2)) else {
        return vec![];
    };
    let d = g.dict.load();
    g.impact(root, &text(a, 0), int(a, 1, 1).clamp(0, 64) as u32)
        .into_iter()
        .map(|r| {
            vec![
                Value::Integer(r.depth as i64),
                s(d.path(r.file)),
                s(d.name(r.scope)),
            ]
        })
        .collect()
}
fn run_neighbors(g: &Graph, a: &[Value]) -> Rows {
    let Some(root) = g.root_id(&text(a, 3)) else {
        return vec![];
    };
    let d = g.dict.load();
    let out = text(a, 2).eq_ignore_ascii_case("out");
    g.neighbors(root, &text(a, 0), int(a, 1, 1).clamp(0, 64) as u32, out)
        .into_iter()
        .map(|(l, n)| vec![Value::Integer(l as i64), s(d.name(n))])
        .collect()
}
fn run_callers(g: &Graph, a: &[Value]) -> Rows {
    let Some(root) = g.root_id(&text(a, 1)) else {
        return vec![];
    };
    let d = g.dict.load();
    g.callers(root, &text(a, 0))
        .into_iter()
        .map(|(f, o)| vec![s(d.path(f)), s(d.name(o))])
        .collect()
}
fn run_callees(g: &Graph, a: &[Value]) -> Rows {
    let Some(root) = g.root_id(&text(a, 1)) else {
        return vec![];
    };
    let d = g.dict.load();
    g.callees(root, &text(a, 0))
        .into_iter()
        .map(|(f, o)| vec![s(d.path(f)), s(d.name(o))])
        .collect()
}

pub static SPECS: [Spec; 4] = [
    Spec { name: c"graph_impact", schema: c"CREATE TABLE x(depth INTEGER, path TEXT, scope TEXT, name HIDDEN, max_depth HIDDEN, root HIDDEN)", ncols: 3, nargs: 3, run: run_impact },
    Spec { name: c"graph_neighbors", schema: c"CREATE TABLE x(depth INTEGER, name TEXT, start HIDDEN, max_depth HIDDEN, dir HIDDEN, root HIDDEN)", ncols: 2, nargs: 4, run: run_neighbors },
    Spec { name: c"graph_callers", schema: c"CREATE TABLE x(path TEXT, scope TEXT, name HIDDEN, root HIDDEN)", ncols: 2, nargs: 2, run: run_callers },
    Spec { name: c"graph_callees", schema: c"CREATE TABLE x(path TEXT, callee TEXT, name HIDDEN, root HIDDEN)", ncols: 2, nargs: 2, run: run_callees },
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
    fn run(&self, args: &[Value]) -> Result<Rows> {
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
        .ok_or_else(|| Error::Module("sqlite-csr-graph needs a file database".into()))?;
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
