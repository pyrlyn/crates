//! In-memory graph: per-root immutable base segments + small append-only delta,
//! published through `ArcSwap` so readers never take a lock.

use crate::segment::{Csr, ENC_EDGES, ENC_IDS, Edge, KIND_IMPORT, KIND_REF};
use arc_swap::ArcSwap;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Immutable base for one root.
#[derive(Debug, Clone)]
pub struct Base {
    /// name -> edges where `name` is referenced (`other` = owner scope) or imported.
    pub users: Csr,
    /// owner scope -> edges to referenced names (`other` = callee).
    pub callees: Csr,
    /// file -> sorted def name ids in that file (non-empty names).
    pub filedefs: Csr,
    /// Highest file generation folded into this base.
    pub generation: i64,
}

impl Base {
    pub fn empty() -> Base {
        Base {
            users: Csr::empty(ENC_EDGES),
            callees: Csr::empty(ENC_EDGES),
            filedefs: Csr::empty(ENC_IDS),
            generation: 0,
        }
    }
    pub fn from_pairs(
        users: Vec<(u32, Edge)>,
        callees: Vec<(u32, Edge)>,
        filedefs: Vec<(u32, u32)>,
        generation: i64,
    ) -> Base {
        let fd = filedefs
            .into_iter()
            .map(|(f, n)| {
                (
                    f,
                    Edge {
                        file: n,
                        kind: 0,
                        other: 0,
                    },
                )
            })
            .collect();
        Base {
            users: Csr::build_edges(ENC_EDGES, users),
            callees: Csr::build_edges(ENC_EDGES, callees),
            filedefs: Csr::build_edges(ENC_IDS, fd),
            generation,
        }
    }
    pub fn bytes(&self) -> usize {
        self.users.bytes().len() + self.callees.bytes().len() + self.filedefs.bytes().len()
    }
}

/// Replacement of all graph data contributed by one file (an upsert or a delete).
#[derive(Debug, Clone, Default)]
pub struct FileOp {
    pub seq: u64,
    pub generation: i64,
    pub file: u32,
    /// (referenced name, edge with file = self.file)
    pub users: Vec<(u32, Edge)>,
    /// (owner scope, edge with other = callee)
    pub callees: Vec<(u32, Edge)>,
    pub defs: Vec<u32>,
}

/// Small mutable-by-copy overlay. Every op supersedes all base data of its file
/// (the file is set in the tombstone bitmap).
#[derive(Debug, Default, Clone)]
pub struct Delta {
    pub ops: Vec<Arc<FileOp>>,
    tomb: Vec<u64>,
    users: HashMap<u32, Vec<Edge>>,
    callees: HashMap<u32, Vec<Edge>>,
    filedefs: HashMap<u32, Vec<u32>>,
    pub edges: usize,
}

impl Delta {
    pub fn from_ops(ops: Vec<Arc<FileOp>>) -> Delta {
        let mut latest: HashMap<u32, Arc<FileOp>> = HashMap::new();
        for op in &ops {
            latest.insert(op.file, op.clone());
        }
        let mut d = Delta {
            ops,
            ..Default::default()
        };
        for op in latest.values() {
            let w = op.file as usize / 64;
            if d.tomb.len() <= w {
                d.tomb.resize(w + 1, 0);
            }
            d.tomb[w] |= 1 << (op.file % 64);
            for (k, e) in &op.users {
                d.users.entry(*k).or_default().push(*e);
            }
            for (k, e) in &op.callees {
                d.callees.entry(*k).or_default().push(*e);
            }
            d.filedefs.insert(op.file, op.defs.clone());
            d.edges += op.users.len() + op.callees.len();
        }
        d
    }
    #[inline]
    pub fn is_tomb(&self, file: u32) -> bool {
        self.tomb
            .get(file as usize / 64)
            .is_some_and(|w| w & (1 << (file % 64)) != 0)
    }
    pub fn max_seq(&self) -> u64 {
        self.ops.iter().map(|o| o.seq).max().unwrap_or(0)
    }
}

/// What readers see: one consistent (base, delta) pair.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub base: Arc<Base>,
    pub delta: Arc<Delta>,
}

impl Snapshot {
    #[inline]
    pub fn users(&self, name: u32, mut f: impl FnMut(Edge)) {
        let d = &self.delta;
        self.base.users.for_each(name, |e| {
            if !d.is_tomb(e.file) {
                f(e)
            }
        });
        if let Some(v) = d.users.get(&name) {
            v.iter().for_each(|e| f(*e));
        }
    }
    #[inline]
    pub fn callees(&self, owner: u32, mut f: impl FnMut(Edge)) {
        let d = &self.delta;
        self.base.callees.for_each(owner, |e| {
            if !d.is_tomb(e.file) {
                f(e)
            }
        });
        if let Some(v) = d.callees.get(&owner) {
            v.iter().for_each(|e| f(*e));
        }
    }
    #[inline]
    pub fn filedefs(&self, file: u32, mut f: impl FnMut(u32)) {
        if self.delta.is_tomb(file) {
            if let Some(v) = self.delta.filedefs.get(&file) {
                v.iter().for_each(|n| f(*n));
            }
        } else {
            self.base.filedefs.for_each(file, |e| f(e.file));
        }
    }
    pub fn file_defines(&self, file: u32, name: u32) -> bool {
        let mut hit = false;
        self.filedefs(file, |n| hit |= n == name);
        hit
    }

    /// Fold base + delta into a fresh immutable base (used by background merge).
    pub fn compact(&self) -> Base {
        let d = &self.delta;
        let mut users = Vec::with_capacity(self.base.users.len_items() + d.edges);
        let mut callees = Vec::with_capacity(self.base.callees.len_items());
        let mut fdefs = Vec::new();
        for i in 0..self.base.users.len_keys() {
            let k = self.base.users.key(i);
            self.base.users.for_each_at(i, |e| {
                if !d.is_tomb(e.file) {
                    users.push((k, e))
                }
            });
        }
        for i in 0..self.base.callees.len_keys() {
            let k = self.base.callees.key(i);
            self.base.callees.for_each_at(i, |e| {
                if !d.is_tomb(e.file) {
                    callees.push((k, e))
                }
            });
        }
        for i in 0..self.base.filedefs.len_keys() {
            let k = self.base.filedefs.key(i);
            if !d.is_tomb(k) {
                self.base
                    .filedefs
                    .for_each_at(i, |e| fdefs.push((k, e.file)));
            }
        }
        for (k, v) in &d.users {
            users.extend(v.iter().map(|e| (*k, *e)));
        }
        for (k, v) in &d.callees {
            callees.extend(v.iter().map(|e| (*k, *e)));
        }
        for (f, v) in &d.filedefs {
            fdefs.extend(v.iter().map(|n| (*f, *n)));
        }
        let generation = d
            .ops
            .iter()
            .map(|o| o.generation)
            .max()
            .unwrap_or(0)
            .max(self.base.generation);
        Base::from_pairs(users, callees, fdefs, generation)
    }
}

/// Interned strings shared by all roots. Append-only; replaced by copy on growth.
#[derive(Debug, Default, Clone)]
pub struct Dict {
    pub names: Vec<Arc<str>>,
    pub name_ids: HashMap<Arc<str>, u32>,
    /// file id -> (root id, path)
    pub files: Vec<Option<(u32, Arc<str>)>>,
    pub file_ids: HashMap<(u32, Arc<str>), u32>,
    pub roots: HashMap<String, u32>,
}

impl Dict {
    pub fn name(&self, id: u32) -> &str {
        self.names.get(id as usize).map(|s| &**s).unwrap_or("")
    }
    pub fn path(&self, file: u32) -> &str {
        self.files
            .get(file as usize)
            .and_then(|f| f.as_ref())
            .map(|(_, p)| &**p)
            .unwrap_or("")
    }
    pub fn set_name(&mut self, id: u32, name: &str) {
        let s: Arc<str> = name.into();
        if self.names.len() <= id as usize {
            self.names.resize(id as usize + 1, "".into());
        }
        self.names[id as usize] = s.clone();
        self.name_ids.insert(s, id);
    }
    pub fn set_file(&mut self, id: u32, root: u32, path: &str) {
        let p: Arc<str> = path.into();
        if self.files.len() <= id as usize {
            self.files.resize(id as usize + 1, None);
        }
        self.files[id as usize] = Some((root, p.clone()));
        self.file_ids.insert((root, p), id);
    }
}

pub type RootCell = ArcSwap<Snapshot>;

/// All roots of one database.
pub struct Graph {
    pub dict: ArcSwap<Dict>,
    pub roots: ArcSwap<HashMap<u32, Arc<RootCell>>>,
    /// Serialises writers (graph publication only; readers never touch it).
    pub writer: Mutex<()>,
    seq: AtomicU64,
    pub merge_threshold: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImpactRow {
    pub depth: u32,
    pub file: u32,
    pub scope: u32,
}

impl Graph {
    pub fn new(dict: Dict, roots: HashMap<u32, Snapshot>) -> Graph {
        Graph {
            dict: ArcSwap::from_pointee(dict),
            roots: ArcSwap::from_pointee(
                roots
                    .into_iter()
                    .map(|(k, v)| (k, Arc::new(ArcSwap::from_pointee(v))))
                    .collect(),
            ),
            writer: Mutex::new(()),
            seq: AtomicU64::new(1),
            merge_threshold: 20_000,
        }
    }

    pub fn snapshot(&self, root: u32) -> Option<Arc<Snapshot>> {
        self.roots.load().get(&root).map(|c| c.load_full())
    }
    pub fn root_id(&self, root: &str) -> Option<u32> {
        self.dict.load().roots.get(root).copied()
    }

    /// Publish a file replacement. Caller must hold `self.writer`.
    pub fn apply_locked(&self, root: u32, mut op: FileOp) -> Arc<Snapshot> {
        op.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let roots = self.roots.load();
        let cell = match roots.get(&root) {
            Some(c) => c.clone(),
            None => {
                let c = Arc::new(ArcSwap::from_pointee(Snapshot {
                    base: Arc::new(Base::empty()),
                    delta: Arc::new(Delta::default()),
                }));
                let mut m = (**roots).clone();
                m.insert(root, c.clone());
                self.roots.store(Arc::new(m));
                c
            }
        };
        let cur = cell.load();
        let mut ops = cur.delta.ops.clone();
        ops.push(Arc::new(op));
        let snap = Arc::new(Snapshot {
            base: cur.base.clone(),
            delta: Arc::new(Delta::from_ops(ops)),
        });
        cell.store(snap.clone());
        snap
    }

    /// Swap in a merged base built from `from`; ops published after `from` stay in the delta.
    pub fn install_merged(&self, root: u32, from: &Snapshot, base: Base) {
        let _w = self.writer.lock().unwrap();
        let roots = self.roots.load();
        let Some(cell) = roots.get(&root) else { return };
        let cur = cell.load();
        let cut = from.delta.max_seq();
        let rest: Vec<_> = cur
            .delta
            .ops
            .iter()
            .filter(|o| o.seq > cut)
            .cloned()
            .collect();
        cell.store(Arc::new(Snapshot {
            base: Arc::new(base),
            delta: Arc::new(Delta::from_ops(rest)),
        }));
    }

    /// rtok `impact` semantics with a global visited set (frontier dedup):
    /// level-by-level BFS from `name`; every (file, owner) reached gets its minimal level.
    pub fn impact(&self, root: u32, name: &str, depth: u32) -> Vec<ImpactRow> {
        let dict = self.dict.load();
        let (Some(snap), Some(&start)) = (self.snapshot(root), dict.name_ids.get(name)) else {
            return vec![];
        };
        if start == 0 {
            return vec![];
        }
        let mut seen: HashMap<(u32, u32), u32> = HashMap::new();
        let mut expanded: HashSet<u32> = HashSet::from([start]);
        let mut frontier = vec![start];
        let mut level = 1;
        while !frontier.is_empty() && level <= depth {
            let mut next = Vec::new();
            for &tip in &frontier {
                let mut add = |file: u32, owner: u32, next: &mut Vec<u32>| {
                    if let std::collections::hash_map::Entry::Vacant(v) = seen.entry((file, owner))
                    {
                        v.insert(level);
                    }
                    if owner != 0 && expanded.insert(owner) {
                        next.push(owner);
                    }
                };
                snap.users(tip, |e| {
                    if e.kind == KIND_IMPORT {
                        snap.filedefs(e.file, |d| add(e.file, d, &mut next));
                    } else {
                        add(e.file, e.other, &mut next);
                    }
                });
            }
            frontier = next;
            level += 1;
        }
        let mut rows: Vec<ImpactRow> = seen
            .into_iter()
            .map(|((file, scope), depth)| ImpactRow { depth, file, scope })
            .collect();
        rows.sort_by(|a, b| {
            (a.depth, dict.path(a.file), dict.name(a.scope)).cmp(&(
                b.depth,
                dict.path(b.file),
                dict.name(b.scope),
            ))
        });
        rows
    }

    /// Distinct names reachable within `depth` hops. `outgoing=false`: who uses me
    /// (owners of refs + defs of importing files); `true`: what I call.
    pub fn neighbors(&self, root: u32, name: &str, depth: u32, outgoing: bool) -> Vec<(u32, u32)> {
        let dict = self.dict.load();
        let (Some(snap), Some(&start)) = (self.snapshot(root), dict.name_ids.get(name)) else {
            return vec![];
        };
        let mut level_of: HashMap<u32, u32> = HashMap::from([(start, 0)]);
        let mut frontier = vec![start];
        for level in 1..=depth {
            let mut next = Vec::new();
            for &n in &frontier {
                let mut push = |m: u32| {
                    if m != 0 && !level_of.contains_key(&m) {
                        level_of.insert(m, level);
                        next.push(m);
                    }
                };
                if outgoing {
                    snap.callees(n, |e| push(e.other));
                } else {
                    snap.users(n, |e| {
                        if e.kind == KIND_REF {
                            push(e.other)
                        } else {
                            snap.filedefs(e.file, &mut push)
                        }
                    });
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        let mut v: Vec<(u32, u32)> = level_of
            .into_iter()
            .filter(|(n, _)| *n != start)
            .map(|(n, l)| (l, n))
            .collect();
        v.sort_by(|a, b| (a.0, dict.name(a.1)).cmp(&(b.0, dict.name(b.1))));
        v
    }

    /// Distinct (file, owner scope) pairs referencing `name` (refs only, no imports).
    pub fn callers(&self, root: u32, name: &str) -> Vec<(u32, u32)> {
        let dict = self.dict.load();
        let (Some(snap), Some(&id)) = (self.snapshot(root), dict.name_ids.get(name)) else {
            return vec![];
        };
        let mut v = vec![];
        snap.users(id, |e| {
            if e.kind == KIND_REF {
                v.push((e.file, e.other))
            }
        });
        v.sort_by(|a, b| (dict.path(a.0), dict.name(a.1)).cmp(&(dict.path(b.0), dict.name(b.1))));
        v.dedup();
        v
    }

    /// Distinct (file, callee) for refs scoped to `name` in files that define `name`.
    pub fn callees(&self, root: u32, name: &str) -> Vec<(u32, u32)> {
        let dict = self.dict.load();
        let (Some(snap), Some(&id)) = (self.snapshot(root), dict.name_ids.get(name)) else {
            return vec![];
        };
        let mut v = vec![];
        snap.callees(id, |e| {
            if e.other != 0 && snap.file_defines(e.file, id) {
                v.push((e.file, e.other))
            }
        });
        v.sort_by(|a, b| (dict.path(a.0), dict.name(a.1)).cmp(&(dict.path(b.0), dict.name(b.1))));
        v.dedup();
        v
    }

    pub fn stats(&self) -> String {
        let mut parts = vec![];
        for (r, c) in self.roots.load().iter() {
            let s = c.load();
            parts.push(format!(
                "{{\"root\":{r},\"base_bytes\":{},\"users_edges\":{},\"callee_edges\":{},\"filedefs\":{},\"delta_ops\":{},\"delta_edges\":{},\"gen\":{}}}",
                s.base.bytes(),
                s.base.users.len_items(),
                s.base.callees.len_items(),
                s.base.filedefs.len_items(),
                s.delta.ops.len(),
                s.delta.edges,
                s.base.generation
            ));
        }
        parts.sort();
        format!("[{}]", parts.join(","))
    }
}

pub fn kind_ref() -> u8 {
    KIND_REF
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Graph").finish_non_exhaustive()
    }
}
