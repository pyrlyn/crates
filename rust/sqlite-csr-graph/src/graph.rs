//! In-memory graph: per-root immutable base segments + small append-only delta,
//! published through `ArcSwap` so readers never take a lock.

use crate::segment::{Csr, ENC_EDGES, ENC_IDS, Edge, KIND_IMPORT, KIND_REF};
use arc_swap::ArcSwap;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
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
    /// What the merge threshold is compared with: delta edges plus 16 per file op.
    pub fn weight(&self) -> usize {
        self.edges + self.ops.len() * 16
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

// ---------------------------------------------------------------- traversal scratch

/// Hasher for the crate's own integer ids (murmur3's 64-bit finalizer). The keys are ids
/// this crate assigned, not attacker-chosen strings, so SipHash's flood resistance buys
/// nothing here and costs most of an insert.
#[derive(Debug, Default, Clone, Copy)]
struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u32(&mut self, n: u32) {
        self.write_u64(n as u64);
    }
    #[inline]
    fn write_u64(&mut self, n: u64) {
        let mut x = self.0 ^ n;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        x ^= x >> 33;
        self.0 = x;
    }
}

type IdSet = HashSet<u64, BuildHasherDefault<IdHasher>>;

#[inline]
fn pack(file: u32, owner: u32) -> u64 {
    ((file as u64) << 32) | owner as u64
}

/// Bitset over dense name ids that remembers the words it set, so clearing it costs the
/// number of visited names, not the size of the id space.
#[derive(Debug, Default)]
struct Bits {
    words: Vec<u64>,
    touched: Vec<u32>,
}

impl Bits {
    /// Set `id`; true if it was clear.
    #[inline]
    fn insert(&mut self, id: u32) -> bool {
        let w = (id / 64) as usize;
        if w >= self.words.len() {
            self.words.resize(w + 1, 0);
        }
        let m = 1u64 << (id % 64);
        let word = &mut self.words[w];
        if *word & m != 0 {
            return false;
        }
        if *word == 0 {
            self.touched.push(w as u32);
        }
        *word |= m;
        true
    }
    fn clear(&mut self) {
        for &w in &self.touched {
            self.words[w as usize] = 0;
        }
        self.touched.clear();
    }
}

/// The buffers one traversal needs. Taken from a per-thread pool and given back when the
/// walk is dropped, so a reader thread allocates them once rather than on every query.
#[derive(Debug, Default)]
struct Scratch {
    /// Names already expanded (or queued for the next level).
    expanded: Bits,
    /// `(file, owner)` pairs already returned (`impact` only).
    seen: IdSet,
    frontier: Vec<u32>,
    next: Vec<u32>,
}

/// Scratch sets kept per thread (one per walk alive at the same time, e.g. a join of two
/// table-valued functions), and the size above which a set is freed instead of kept.
const POOL_PER_THREAD: usize = 4;
const KEEP_SEEN: usize = 1 << 20;
const KEEP_WORDS: usize = 1 << 16;

thread_local! {
    static POOL: RefCell<Vec<Scratch>> = const { RefCell::new(Vec::new()) };
}

impl Scratch {
    fn take() -> Scratch {
        POOL.try_with(|p| p.try_borrow_mut().ok().and_then(|mut p| p.pop()))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn give(mut self) {
        if self.seen.capacity() > KEEP_SEEN || self.expanded.words.len() > KEEP_WORDS {
            return;
        }
        self.expanded.clear();
        self.seen.clear();
        self.frontier.clear();
        self.next.clear();
        // During thread teardown the pool may be gone; the scratch is then just dropped.
        let _ = POOL.try_with(move |p| {
            if let Ok(mut p) = p.try_borrow_mut()
                && p.len() < POOL_PER_THREAD
            {
                p.push(self);
            }
        });
    }
}

/// Level-by-level `impact` from one or more start names. Each [`ImpactWalk::next_level`]
/// expands one more hop, so a caller that stops early (a SQL `LIMIT`) never pays for the
/// deeper levels. Holds its snapshot, so writers and merges never wait for it.
#[derive(Debug)]
pub struct ImpactWalk {
    snap: Arc<Snapshot>,
    dict: Arc<Dict>,
    depth: u32,
    level: u32,
    s: Scratch,
    #[cfg(feature = "parallel")]
    par_min: usize,
}

/// Frontier size from which the `parallel` feature expands a level on rayon's pool:
/// below it, splitting the work costs more than it saves.
#[cfg(feature = "parallel")]
pub const DEFAULT_PARALLEL_MIN_FRONTIER: usize = 256;

impl ImpactWalk {
    fn new(snap: Arc<Snapshot>, dict: Arc<Dict>, starts: &[u32], depth: u32) -> ImpactWalk {
        let mut s = Scratch::take();
        for &start in starts {
            if start != 0 && s.expanded.insert(start) {
                s.frontier.push(start);
            }
        }
        ImpactWalk {
            snap,
            dict,
            depth,
            level: 0,
            s,
            #[cfg(feature = "parallel")]
            par_min: DEFAULT_PARALLEL_MIN_FRONTIER,
        }
    }

    /// The dictionary the rows' ids resolve against.
    pub fn dict(&self) -> &Dict {
        &self.dict
    }

    /// Levels expanded so far.
    pub fn level(&self) -> u32 {
        self.level
    }

    /// The rows first reached at the next level, sorted by (path, scope); `None` once the
    /// walk reached `depth` or ran out of names to expand.
    pub fn next_level(&mut self) -> Option<Vec<ImpactRow>> {
        while !self.s.frontier.is_empty() && self.level < self.depth {
            self.level += 1;
            let rows = self.expand();
            if !rows.is_empty() {
                return Some(rows);
            }
        }
        None
    }

    fn expand(&mut self) -> Vec<ImpactRow> {
        let level = self.level;
        let snap = &*self.snap;
        #[cfg(feature = "parallel")]
        let batches = (self.s.frontier.len() >= self.par_min).then(|| self.collect_parallel());
        #[cfg(not(feature = "parallel"))]
        let batches: Option<Vec<Vec<(u32, u32)>>> = None;
        #[cfg(feature = "parallel")]
        let par = batches.is_some();
        let Scratch {
            expanded,
            seen,
            frontier,
            next,
        } = &mut self.s;
        let mut rows = Vec::new();
        let mut add = |file: u32, owner: u32| {
            if seen.insert(pack(file, owner)) {
                rows.push(ImpactRow {
                    depth: level,
                    file,
                    scope: owner,
                });
            }
            if owner != 0 && expanded.insert(owner) {
                next.push(owner);
            }
        };
        match batches {
            // Same pairs in the same order as the sequential loop, so the rows and the next
            // frontier are identical.
            Some(b) => b.into_iter().flatten().for_each(|(f, o)| add(f, o)),
            None => {
                for &tip in frontier.iter() {
                    users_of(snap, tip, &mut add);
                }
            }
        }
        std::mem::swap(frontier, next);
        next.clear();
        let d = &*self.dict;
        let by_path_scope = |a: &ImpactRow, b: &ImpactRow| {
            (d.path(a.file), d.name(a.scope)).cmp(&(d.path(b.file), d.name(b.scope)))
        };
        #[cfg(feature = "parallel")]
        if par {
            rayon::slice::ParallelSliceMut::par_sort_unstable_by(&mut rows[..], by_path_scope);
            return rows;
        }
        rows.sort_unstable_by(by_path_scope);
        rows
    }

    /// Every use of the frontier's names in frontier order, decoded on rayon's pool.
    /// Pairs returned at an earlier level are dropped here already: they add no row and
    /// their owner was expanded then.
    #[cfg(feature = "parallel")]
    fn collect_parallel(&self) -> Vec<Vec<(u32, u32)>> {
        use rayon::prelude::*;
        let (snap, seen) = (&*self.snap, &self.s.seen);
        let chunk = (self.s.frontier.len() / (rayon::current_num_threads() * 4)).max(16);
        self.s
            .frontier
            .par_chunks(chunk)
            .map(|tips| {
                let mut out = Vec::new();
                for &tip in tips {
                    users_of(snap, tip, |f, o| {
                        if !seen.contains(&pack(f, o)) {
                            out.push((f, o));
                        }
                    });
                }
                out
            })
            .collect()
    }
}

impl Drop for ImpactWalk {
    fn drop(&mut self) {
        std::mem::take(&mut self.s).give();
    }
}

/// `(file, owner)` for every use of `name`: the owner scope of a reference, or each name
/// defined in a file that imports `name`.
#[inline]
fn users_of(snap: &Snapshot, name: u32, mut f: impl FnMut(u32, u32)) {
    snap.users(name, |e| {
        if e.kind == KIND_IMPORT {
            snap.filedefs(e.file, |d| f(e.file, d));
        } else {
            f(e.file, e.other);
        }
    });
}

/// Level-by-level `neighbors`: distinct names one more hop away per
/// [`NeighborsWalk::next_level`].
#[derive(Debug)]
pub struct NeighborsWalk {
    snap: Arc<Snapshot>,
    dict: Arc<Dict>,
    depth: u32,
    level: u32,
    outgoing: bool,
    s: Scratch,
}

impl NeighborsWalk {
    /// The dictionary the rows' ids resolve against.
    pub fn dict(&self) -> &Dict {
        &self.dict
    }

    /// `(level, name)` for the names first reached at the next level, sorted by name.
    pub fn next_level(&mut self) -> Option<Vec<(u32, u32)>> {
        while !self.s.frontier.is_empty() && self.level < self.depth {
            self.level += 1;
            let snap = &*self.snap;
            let Scratch {
                expanded,
                frontier,
                next,
                ..
            } = &mut self.s;
            let mut push = |m: u32| {
                if m != 0 && expanded.insert(m) {
                    next.push(m);
                }
            };
            for &n in frontier.iter() {
                if self.outgoing {
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
            std::mem::swap(frontier, next);
            next.clear();
            if !frontier.is_empty() {
                let d = &*self.dict;
                let mut v: Vec<u32> = frontier.clone();
                v.sort_unstable_by(|a, b| d.name(*a).cmp(d.name(*b)));
                let level = self.level;
                return Some(v.into_iter().map(|n| (level, n)).collect());
            }
        }
        None
    }
}

impl Drop for NeighborsWalk {
    fn drop(&mut self) {
        std::mem::take(&mut self.s).give();
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
    #[cfg(feature = "parallel")]
    parallel_min_frontier: std::sync::atomic::AtomicUsize,
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
            #[cfg(feature = "parallel")]
            parallel_min_frontier: DEFAULT_PARALLEL_MIN_FRONTIER.into(),
        }
    }

    /// Expand a BFS level on rayon's pool once its frontier has at least `n` names
    /// (default [`DEFAULT_PARALLEL_MIN_FRONTIER`]; `usize::MAX` keeps every walk on the
    /// calling thread). Applies to walks started afterwards.
    #[cfg(feature = "parallel")]
    pub fn set_parallel_min_frontier(&self, n: usize) {
        self.parallel_min_frontier.store(n, Ordering::Relaxed);
    }

    /// See [`Graph::set_parallel_min_frontier`].
    #[cfg(feature = "parallel")]
    pub fn parallel_min_frontier(&self) -> usize {
        self.parallel_min_frontier.load(Ordering::Relaxed)
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
    /// Returns false (and changes nothing) if another merge installed a base since `from`
    /// was taken: that base already holds more, and installing this one would drop the ops
    /// the other merge folded. They stay in the delta for the next merge.
    pub fn install_merged(&self, root: u32, from: &Snapshot, base: Base) -> bool {
        let _w = self.writer.lock().unwrap();
        let roots = self.roots.load();
        let Some(cell) = roots.get(&root) else {
            return false;
        };
        let cur = cell.load();
        if !Arc::ptr_eq(&cur.base, &from.base) {
            return false;
        }
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
        true
    }

    /// rtok `impact` semantics with a global visited set (frontier dedup):
    /// level-by-level BFS from `name`; every (file, owner) reached gets its minimal level.
    /// Rows are sorted by (depth, path, scope).
    pub fn impact(&self, root: u32, name: &str, depth: u32) -> Vec<ImpactRow> {
        self.impact_many(root, &[name], depth)
    }

    /// [`Graph::impact`] from several start names at once (e.g. every symbol a change
    /// touched): one BFS with all of them at level 0, so each (file, owner) gets its
    /// smallest depth from any start and shared callers are expanded once.
    pub fn impact_many(&self, root: u32, names: &[&str], depth: u32) -> Vec<ImpactRow> {
        let Some(mut walk) = self.impact_walk(root, names, depth) else {
            return vec![];
        };
        let mut rows = vec![];
        while let Some(level) = walk.next_level() {
            rows.extend(level);
        }
        rows
    }

    /// The lazy form of [`Graph::impact_many`]; `None` for an unknown root.
    pub fn impact_walk(&self, root: u32, names: &[&str], depth: u32) -> Option<ImpactWalk> {
        // Snapshot first: the writer publishes the dictionary before the delta, so a
        // dictionary loaded afterwards knows every id the snapshot holds.
        let snap = self.snapshot(root)?;
        let dict = self.dict.load_full();
        let starts: Vec<u32> = names
            .iter()
            .filter_map(|n| dict.name_ids.get(*n).copied())
            .collect();
        #[cfg_attr(not(feature = "parallel"), allow(unused_mut))]
        let mut walk = ImpactWalk::new(snap, dict, &starts, depth);
        #[cfg(feature = "parallel")]
        {
            walk.par_min = self.parallel_min_frontier();
        }
        Some(walk)
    }

    /// Distinct names reachable within `depth` hops. `outgoing=false`: who uses me
    /// (owners of refs + defs of importing files); `true`: what I call.
    pub fn neighbors(&self, root: u32, name: &str, depth: u32, outgoing: bool) -> Vec<(u32, u32)> {
        let Some(mut walk) = self.neighbors_walk(root, name, depth, outgoing) else {
            return vec![];
        };
        let mut v = vec![];
        while let Some(level) = walk.next_level() {
            v.extend(level);
        }
        v
    }

    /// The lazy form of [`Graph::neighbors`]; `None` for an unknown root or name.
    pub fn neighbors_walk(
        &self,
        root: u32,
        name: &str,
        depth: u32,
        outgoing: bool,
    ) -> Option<NeighborsWalk> {
        let snap = self.snapshot(root)?;
        let dict = self.dict.load_full();
        let start = *dict.name_ids.get(name)?;
        let mut s = Scratch::take();
        s.expanded.insert(start);
        s.frontier.push(start);
        Some(NeighborsWalk {
            snap,
            dict,
            depth,
            level: 0,
            outgoing,
            s,
        })
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
