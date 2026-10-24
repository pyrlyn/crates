//! Table-valued functions. Traversal runs in Rust over the shared lock-free graph;
//! SQLite only sees the result rows.
//!
//! - `graph_impact(name, depth, root)`        -> depth, path, scope
//! - `graph_neighbors(name, depth, dir, root)` -> depth, name   (dir: 'in' | 'out')
//! - `graph_callers(name, root)`              -> path, scope
//! - `graph_callees(name, root)`              -> path, callee
//!
//! Scalars: `graph_stats()`, `graph_merge(root)`, `graph_reload()`.

use crate::graph::Graph;
use crate::store::{self, Result};
use rusqlite::ffi;
use rusqlite::functions::FunctionFlags;
use rusqlite::types::Value;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor,
};
use rusqlite::{Connection, Error};
use std::borrow::Cow;
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

pub struct Aux {
    spec: &'static Spec,
    graph: GraphRef,
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

#[repr(C)]
pub struct Tvf {
    base: ffi::sqlite3_vtab,
    spec: &'static Spec,
    graph: GraphRef,
}

#[allow(unsafe_code)]
unsafe impl<'vtab> VTab<'vtab> for Tvf {
    type Aux = Aux;
    type Cursor = TvfCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Aux>,
        _m: &[u8],
        _d: &[u8],
        _t: &[u8],
        _a: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        let aux = aux.ok_or_else(|| Error::ModuleError("missing aux".into()))?;
        db.config(VTabConfig::Innocuous)?;
        Ok((
            Cow::Borrowed(aux.spec.schema),
            Tvf {
                base: ffi::sqlite3_vtab::default(),
                spec: aux.spec,
                graph: aux.graph.clone(),
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut slot: Vec<Option<usize>> = vec![None; self.spec.nargs as usize];
        for (i, c) in info.constraints().enumerate() {
            let col = c.column() - self.spec.ncols;
            if col < 0 || col >= self.spec.nargs {
                continue;
            }
            if c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ {
                if !c.is_usable() {
                    return Ok(false);
                }
                slot[col as usize] = Some(i);
            }
        }
        let mut mask = 0;
        let mut n = 0;
        for (a, s) in slot.iter().enumerate() {
            if let Some(i) = s {
                n += 1;
                mask |= 1 << a;
                let mut u = info.constraint_usage(*i);
                u.set_argv_index(n);
                u.set_omit(true);
            }
        }
        info.set_idx_num(mask);
        info.set_estimated_cost(if mask & 1 != 0 { 10.0 } else { 1e12 });
        info.set_estimated_rows(100);
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<TvfCursor<'vtab>> {
        Ok(TvfCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            tab: self,
            rows: vec![],
            pos: 0,
        })
    }
}

#[repr(C)]
pub struct TvfCursor<'vtab> {
    base: ffi::sqlite3_vtab_cursor,
    tab: &'vtab Tvf,
    rows: Rows,
    pos: usize,
}

#[allow(unsafe_code)]
unsafe impl VTabCursor for TvfCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _s: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let mut a = vec![Value::Null; self.tab.spec.nargs as usize];
        let mut j = 0;
        for (i, slot) in a.iter_mut().enumerate() {
            if idx_num & (1 << i) != 0 {
                *slot = args.get::<Value>(j)?;
                j += 1;
            }
        }
        let g = self.tab.graph.get()?;
        self.rows = (self.tab.spec.run)(&g, &a);
        self.pos = 0;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.pos += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.pos >= self.rows.len()
    }
    fn column(&self, ctx: &mut Context, i: c_int) -> Result<()> {
        match self.rows[self.pos].get(i as usize) {
            Some(v) => ctx.set_result(v),
            None => ctx.set_result(&Value::Null),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.pos as i64)
    }
}

const MODULE: Module<'static, Tvf> = Module::eponymous_only_module();

fn register_impl(conn: &Connection, graph: GraphRef) -> Result<()> {
    for spec in SPECS.iter() {
        conn.create_module(
            spec.name,
            &MODULE,
            Some(Aux {
                spec,
                graph: graph.clone(),
            }),
        )?;
    }
    let g1 = graph.clone();
    conn.create_scalar_function("graph_stats", 0, FunctionFlags::SQLITE_UTF8, move |_| {
        Ok(g1
            .get()
            .map_err(|e| Error::UserFunctionError(Box::new(e)))?
            .stats())
    })?;
    let path = match &graph {
        GraphRef::Lazy(p, _) => Some(p.clone()),
        GraphRef::Shared(_) => conn.path().map(PathBuf::from),
    };
    let (g2, p2) = (graph.clone(), path.clone());
    conn.create_scalar_function("graph_merge", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let root: String = ctx.get(0)?;
        let g = g2
            .get()
            .map_err(|e| Error::UserFunctionError(Box::new(e)))?;
        let Some(rid) = g.root_id(&root) else {
            return Ok(0);
        };
        let p = p2
            .clone()
            .ok_or_else(|| Error::UserFunctionError("in-memory database".into()))?;
        store::merge_root(&g, &p, rid).map_err(|e| Error::UserFunctionError(Box::new(e)))?;
        Ok(1)
    })?;
    conn.create_scalar_function("graph_reload", 0, FunctionFlags::SQLITE_UTF8, move |_| {
        let p = path
            .clone()
            .ok_or_else(|| Error::UserFunctionError("in-memory database".into()))?;
        store::graph_for(&p, true).map_err(|e| Error::UserFunctionError(Box::new(e)))?;
        Ok(1)
    })?;
    Ok(())
}

/// Register on a connection; the graph is loaded lazily (once per process per DB file).
pub fn register(conn: &Connection) -> Result<()> {
    let p = conn
        .path()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Error::ModuleError("sqlite-csr-graph needs a file database".into()))?;
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

impl std::fmt::Debug for Aux {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aux").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Tvf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tvf").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for TvfCursor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvfCursor").finish_non_exhaustive()
    }
}
