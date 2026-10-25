// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Approximate nearest-neighbour search: an HNSW graph persisted in one SQLite file,
//! with optional `i8` quantization.
//!
//! ```no_run
//! use sqlite_hnsw::{Params, Quant, VecDb};
//! let db = VecDb::open("vectors.db", 3, Params { quant: Quant::I8, ..Params::default() })?;
//! db.insert(1, &[0.1, 0.2, 0.3])?;
//! db.save()?;
//! let hits = db.search(&[0.1, 0.2, 0.3], 10);
//! # Ok::<(), sqlite_hnsw::Error>(())
//! ```
//!
//! * Graph: HNSW (Malkov & Yashunin) with the neighbour-selection heuristic;
//!   `m`, `ef_construction` and `ef_search` are tunable.
//! * Vectors: cosine on normalized vectors, stored as contiguous `f32` or as
//!   `i8` with one `f32` scale per vector.
//! * Concurrency: nodes live in a lock-free append-only arena (doubling
//!   buckets), every adjacency list has its own `RwLock`, so searches never take
//!   a global lock and inserts only lock the lists they rewrite.
//! * Persistence: nodes are serialized into 1024-node BLOB pages in one SQLite
//!   table; only pages touched since the last `save` are rewritten. Loading uses
//!   SQLite's mmap I/O and parses pages straight into the arena.
//! * Distance: [`distance`] holds the dot-product kernels; the `simd` feature adds NEON
//!   (aarch64) and runtime-detected AVX2 + FMA (x86_64) versions.

pub mod distance;
#[cfg(feature = "simd")]
#[allow(unsafe_code)]
mod simd;
#[allow(unsafe_code)]
mod sqlite;

use distance::{dot, dot_i8};

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashSet};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AO};

use parking_lot::{Mutex, RwLock};

/// Any failure: SQLite errors, a dimension mismatch, a corrupt page.
#[derive(Debug)]
pub struct Error(
    /// Human-readable message.
    pub String,
);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
/// `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// How vectors are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quant {
    /// Full `f32` components.
    F32,
    /// One `i8` per component plus one `f32` scale per vector (about 4x smaller).
    I8,
    /// [`Quant::I8`] for the graph walk, plus the `f32` vector (in memory and on disk) to
    /// rescore the best `k * rerank` candidates. About 1.25x the size of [`Quant::F32`].
    I8Rerank,
}

impl Quant {
    fn code(self) -> i64 {
        match self {
            Quant::F32 => 0,
            Quant::I8 => 1,
            Quant::I8Rerank => 2,
        }
    }
    fn from_code(c: i64) -> Result<Self> {
        match c {
            0 => Ok(Quant::F32),
            1 => Ok(Quant::I8),
            2 => Ok(Quant::I8Rerank),
            _ => Err(Error(format!("unknown quantization code {c}"))),
        }
    }
}

/// Index parameters.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Neighbours per node per layer (layer 0 keeps `2 * m`).
    pub m: usize,
    /// Candidate list size while inserting.
    pub ef_construction: usize,
    /// Candidate list size while searching (raise for recall, lower for speed).
    pub ef_search: usize,
    /// Vector storage.
    pub quant: Quant,
    /// With [`Quant::I8Rerank`]: rescore the best `k * rerank` `i8` candidates with the
    /// `f32` vectors (`0` counts as `1`). Ignored by the other modes; not stored in the file.
    pub rerank: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            m: 16,
            ef_construction: 200,
            ef_search: 64,
            quant: Quant::F32,
            rerank: 4,
        }
    }
}

enum Vector {
    F32(Box<[f32]>),
    I8(Box<[i8]>, f32),
    /// `i8` codes, scale, and the normalized `f32` vector for rescoring.
    I8Rerank(Box<[i8]>, f32, Box<[f32]>),
}

struct Node {
    id: u64,
    vec: Vector,
    links: Box<[RwLock<Vec<u32>>]>,
    /// Tombstone: still walked through, never returned; dropped by `compact`.
    deleted: AtomicBool,
}

const PAGE: usize = 1024;
const BASE: usize = 1024;
const BUCKETS: usize = 40;

/// Append-only arena with doubling buckets: `get` is lock-free.
struct Arena {
    buckets: [OnceLock<Box<[OnceLock<Node>]>>; BUCKETS],
    len: AtomicUsize,
}

fn locate(i: usize) -> (usize, usize) {
    let x = i + BASE;
    let b = (usize::BITS - 1 - x.leading_zeros()) as usize - BASE.trailing_zeros() as usize;
    (b, x - (BASE << b))
}

impl Arena {
    fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| OnceLock::new()),
            len: AtomicUsize::new(0),
        }
    }
    fn push(&self, n: Node) -> u32 {
        let i = self.len.fetch_add(1, AO::AcqRel);
        let (b, o) = locate(i);
        let bucket =
            self.buckets[b].get_or_init(|| (0..BASE << b).map(|_| OnceLock::new()).collect());
        if bucket[o].set(n).is_err() {
            unreachable!("slot reserved twice");
        }
        i as u32
    }
    fn get(&self, i: u32) -> Option<&Node> {
        let (b, o) = locate(i as usize);
        self.buckets[b].get()?[o].get()
    }
    fn len(&self) -> usize {
        self.len.load(AO::Acquire)
    }
    /// Every node in index order; needs ownership, so no push can be in flight.
    fn into_nodes(self) -> impl Iterator<Item = Node> {
        self.buckets
            .into_iter()
            .filter_map(OnceLock::into_inner)
            .flat_map(|b| b.into_vec().into_iter().filter_map(OnceLock::into_inner))
    }
}

#[derive(Clone, Copy, PartialEq)]
struct D(f32, u32);
impl Eq for D {}
impl PartialOrd for D {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for D {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0).then(self.1.cmp(&o.1))
    }
}

thread_local! {
    static VISITED: RefCell<(Vec<u32>, u32)> = const { RefCell::new((Vec::new(), 0)) };
}

fn normalize(v: &mut [f32]) {
    let n = dot(v, v).sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Runs `f` on every item: on the rayon pool with `parallel`, in order without it.
#[cfg(feature = "parallel")]
fn each<T: Send>(items: Vec<T>, f: impl Fn(T) + Send + Sync) {
    use rayon::prelude::*;
    items.into_par_iter().for_each(f);
}
#[cfg(not(feature = "parallel"))]
fn each<T>(items: Vec<T>, f: impl Fn(T)) {
    items.into_iter().for_each(f);
}

fn quantize(v: &[f32]) -> (Box<[i8]>, f32) {
    let max = v
        .iter()
        .fold(0f32, |m, x| m.max(x.abs()))
        .max(f32::MIN_POSITIVE);
    let s = max / 127.0;
    (v.iter().map(|x| (x / s).round() as i8).collect(), s)
}

impl Vector {
    fn encode(v: &[f32], q: Quant) -> Self {
        match q {
            Quant::F32 => Vector::F32(v.into()),
            Quant::I8 => {
                let (c, s) = quantize(v);
                Vector::I8(c, s)
            }
            Quant::I8Rerank => {
                let (c, s) = quantize(v);
                Vector::I8Rerank(c, s, v.into())
            }
        }
    }
    fn decode(&self) -> Vec<f32> {
        match self {
            Vector::F32(v) | Vector::I8Rerank(_, _, v) => v.to_vec(),
            Vector::I8(v, s) => v.iter().map(|x| *x as f32 * s).collect(),
        }
    }
    /// Cosine distance (`1 - dot`) to a normalized `f32` query, as the graph walk sees it.
    #[inline]
    fn dist(&self, q: &[f32]) -> f32 {
        1.0 - match self {
            Vector::F32(v) => dot(q, v),
            Vector::I8(v, s) | Vector::I8Rerank(v, s, _) => dot_i8(q, v) * s,
        }
    }
    /// Distance from the most precise stored form: the `f32` copy when there is one.
    #[inline]
    fn exact_dist(&self, q: &[f32]) -> f32 {
        match self {
            Vector::I8Rerank(_, _, v) => 1.0 - dot(q, v),
            _ => self.dist(q),
        }
    }
}

/// The index: in memory for search, persisted to SQLite by [`VecDb::save`].
pub struct VecDb {
    dim: usize,
    params: Params,
    ef_search: AtomicUsize,
    rerank: AtomicUsize,
    arena: Arena,
    /// (entry node, its level); `u64::MAX` when empty.
    entry: RwLock<u64>,
    dirty: Mutex<BTreeSet<u32>>,
    /// Nodes tombstoned in memory since the last save.
    new_tombs: Mutex<Vec<u32>>,
    /// Tombstoned nodes still in the arena.
    deleted: AtomicUsize,
    /// Set by `compact`: the next save replaces every page and tombstone row.
    rewrite: AtomicBool,
    rng: AtomicU64,
    db: Mutex<sqlite::Db>,
}

impl std::fmt::Debug for VecDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VecDb")
            .field("dim", &self.dim)
            .field("params", &self.params)
            .field("len", &self.len())
            .field("tombstones", &self.tombstones())
            .finish_non_exhaustive()
    }
}

fn pack(node: u32, level: usize) -> u64 {
    (u64::from(node) << 8) | level as u64
}
fn unpack(e: u64) -> (u32, usize) {
    ((e >> 8) as u32, (e & 0xff) as usize)
}

impl VecDb {
    /// Opens (or creates) the store at `path`. Existing files keep their stored
    /// `m`/`ef_construction`/quantization; `params.ef_search` always applies.
    pub fn open(path: &str, dim: usize, params: Params) -> Result<Self> {
        let db = sqlite::Db::open(path)?;
        db.exec(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA mmap_size=17179869184;
             PRAGMA page_size=16384;
             CREATE TABLE IF NOT EXISTS vdb_meta (k INTEGER PRIMARY KEY, v INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS vdb_page (no INTEGER PRIMARY KEY, data BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS vdb_tomb (node INTEGER PRIMARY KEY);",
        )?;
        let mut me = Self {
            dim,
            params,
            ef_search: AtomicUsize::new(params.ef_search),
            rerank: AtomicUsize::new(params.rerank),
            arena: Arena::new(),
            entry: RwLock::new(u64::MAX),
            dirty: Mutex::new(BTreeSet::new()),
            new_tombs: Mutex::new(Vec::new()),
            deleted: AtomicUsize::new(0),
            rewrite: AtomicBool::new(false),
            rng: AtomicU64::new(0x9E37_79B9_7F4A_7C15),
            db: Mutex::new(db),
        };
        me.load()?;
        Ok(me)
    }

    /// Number of stored vectors, tombstoned ones included until [`compact`](Self::compact).
    pub fn len(&self) -> usize {
        self.arena.len()
    }
    /// Number of tombstoned vectors (deleted, not yet compacted away).
    pub fn tombstones(&self) -> usize {
        self.deleted.load(AO::Relaxed)
    }
    /// `true` when no vector is stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Changes the search candidate list size for later searches.
    pub fn set_ef_search(&self, ef: usize) {
        self.ef_search.store(ef, AO::Relaxed);
    }
    /// Changes the rerank factor ([`Params::rerank`]) for later searches.
    pub fn set_rerank(&self, factor: usize) {
        self.rerank.store(factor, AO::Relaxed);
    }

    fn load(&mut self) -> Result<()> {
        let db = self.db.lock();
        let mut meta = [0i64; 6];
        let mut found = false;
        {
            let mut st = db.prepare("SELECT k, v FROM vdb_meta")?;
            while st.step()? {
                found = true;
                if let Some(m) = meta.get_mut(st.col_i64(0) as usize) {
                    *m = st.col_i64(1);
                }
            }
        }
        if !found {
            drop(db);
            return Ok(());
        }
        if meta[0] as usize != self.dim {
            return Err(Error(format!("stored dim {} != {}", meta[0], self.dim)));
        }
        self.params.m = meta[1] as usize;
        self.params.ef_construction = meta[2] as usize;
        self.params.quant = Quant::from_code(meta[3])?;
        *self.entry.get_mut() = meta[4] as u64;
        {
            let mut st = db.prepare("SELECT data FROM vdb_page ORDER BY no")?;
            while st.step()? {
                let mut b = st.col_blob(0);
                while !b.is_empty() {
                    let n = self.read_node(&mut b)?;
                    self.arena.push(n);
                }
            }
        }
        let mut st = db.prepare("SELECT node FROM vdb_tomb")?;
        while st.step()? {
            let i = u32::try_from(st.col_i64(0)).ok();
            if let Some(n) = i.and_then(|i| self.arena.get(i))
                && !n.deleted.swap(true, AO::Relaxed)
            {
                *self.deleted.get_mut() += 1;
            }
        }
        Ok(())
    }

    fn read_node(&self, b: &mut &[u8]) -> Result<Node> {
        fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
            if b.len() < n {
                return Err(Error("truncated page".into()));
            }
            let (h, t) = b.split_at(n);
            *b = t;
            Ok(h)
        }
        let id = u64::from_le_bytes(take(b, 8)?.try_into().unwrap_or_default());
        let levels = take(b, 1)?[0] as usize + 1;
        let f32s = |b: &mut &[u8]| -> Result<Box<[f32]>> {
            Ok(take(b, self.dim * 4)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect())
        };
        let i8s = |b: &mut &[u8]| -> Result<(Box<[i8]>, f32)> {
            let s = f32::from_le_bytes(take(b, 4)?.try_into().unwrap_or_default());
            Ok((take(b, self.dim)?.iter().map(|x| *x as i8).collect(), s))
        };
        let vec = match self.params.quant {
            Quant::F32 => Vector::F32(f32s(b)?),
            Quant::I8 => {
                let (c, s) = i8s(b)?;
                Vector::I8(c, s)
            }
            Quant::I8Rerank => {
                let (c, s) = i8s(b)?;
                Vector::I8Rerank(c, s, f32s(b)?)
            }
        };
        let mut links = Vec::with_capacity(levels);
        for _ in 0..levels {
            let n = u16::from_le_bytes(take(b, 2)?.try_into().unwrap_or_default()) as usize;
            links.push(RwLock::new(
                take(b, n * 4)?
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_le_bytes(*c))
                    .collect(),
            ));
        }
        Ok(Node {
            id,
            vec,
            links: links.into(),
            deleted: AtomicBool::new(false),
        })
    }

    fn write_node(n: &Node, out: &mut Vec<u8>) {
        out.extend_from_slice(&n.id.to_le_bytes());
        out.push((n.links.len() - 1) as u8);
        let f32s = |out: &mut Vec<u8>, v: &[f32]| {
            v.iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes()))
        };
        let i8s = |out: &mut Vec<u8>, v: &[i8], s: f32| {
            out.extend_from_slice(&s.to_le_bytes());
            out.extend(v.iter().map(|x| *x as u8));
        };
        match &n.vec {
            Vector::F32(v) => f32s(out, v),
            Vector::I8(v, s) => i8s(out, v, *s),
            Vector::I8Rerank(v, s, f) => {
                i8s(out, v, *s);
                f32s(out, f);
            }
        }
        for l in n.links.iter() {
            let l = l.read();
            out.extend_from_slice(&(l.len() as u16).to_le_bytes());
            l.iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
        }
    }

    /// Writes every page touched and every tombstone set since the last save in one
    /// transaction. Returns the number of pages written. On failure the transaction is
    /// rolled back and the pending pages stay pending for the next save.
    pub fn save(&self) -> Result<usize> {
        let pages: Vec<u32> = std::mem::take(&mut *self.dirty.lock())
            .into_iter()
            .collect();
        let tombs = std::mem::take(&mut *self.new_tombs.lock());
        let rewrite = self.rewrite.load(AO::Acquire);
        let db = self.db.lock();
        if let Err(e) = self.write(&db, &pages, &tombs, rewrite) {
            let _ = db.exec("ROLLBACK");
            self.dirty.lock().extend(pages);
            self.new_tombs.lock().extend(tombs);
            return Err(e);
        }
        self.rewrite.store(false, AO::Release);
        db.exec("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(pages.len())
    }

    fn write(&self, db: &sqlite::Db, pages: &[u32], tombs: &[u32], rewrite: bool) -> Result<()> {
        db.exec("BEGIN IMMEDIATE")?;
        if rewrite {
            db.exec("DELETE FROM vdb_page; DELETE FROM vdb_tomb;")?;
        }
        {
            let mut st = db.prepare("INSERT OR IGNORE INTO vdb_tomb (node) VALUES (?)")?;
            for &t in tombs {
                st.bind_i64(1, i64::from(t))?;
                st.step()?;
                st.reset();
            }
            let mut st = db.prepare("INSERT OR REPLACE INTO vdb_page (no, data) VALUES (?, ?)")?;
            let mut buf = Vec::new();
            for &p in pages {
                buf.clear();
                let start = p as usize * PAGE;
                for i in start..(start + PAGE).min(self.len()) {
                    if let Some(n) = self.arena.get(i as u32) {
                        Self::write_node(n, &mut buf);
                    }
                }
                st.bind_i64(1, i64::from(p))?;
                st.bind_blob(2, &buf)?;
                st.step()?;
                st.reset();
            }
            let mut st = db.prepare("INSERT OR REPLACE INTO vdb_meta (k, v) VALUES (?, ?)")?;
            let meta = [
                self.dim as i64,
                self.params.m as i64,
                self.params.ef_construction as i64,
                self.params.quant.code(),
                *self.entry.read() as i64,
                self.len() as i64,
            ];
            for (k, v) in meta.into_iter().enumerate() {
                st.bind_i64(1, k as i64)?;
                st.bind_i64(2, v)?;
                st.step()?;
                st.reset();
            }
        }
        db.exec("COMMIT")
    }

    fn node(&self, i: u32) -> &Node {
        self.arena.get(i).expect("linked node is published")
    }

    fn random_level(&self) -> usize {
        // xorshift64*; good enough for level assignment.
        let mut x = self.rng.fetch_add(0x9E37_79B9_7F4A_7C15, AO::Relaxed);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let u = ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
        let ml = 1.0 / (self.params.m as f64).ln();
        ((-u.ln() * ml) as usize).min(15)
    }

    fn search_layer(&self, q: &[f32], eps: &[D], ef: usize, level: usize) -> Vec<D> {
        self.search_layer_if(q, eps, ef, level, None)
    }

    /// Beam search on one layer. With `accept`, rejected nodes are still walked through
    /// but never enter the result list (so the beam does not stop early on them).
    fn search_layer_if(
        &self,
        q: &[f32],
        eps: &[D],
        ef: usize,
        level: usize,
        accept: Option<&dyn Fn(u32) -> bool>,
    ) -> Vec<D> {
        let ok = |i: u32| accept.is_none_or(|f| f(i));
        VISITED.with(|v| {
            let (marks, gen_) = &mut *v.borrow_mut();
            let n = self.len();
            if marks.len() < n {
                marks.resize(n + n / 4 + 1024, 0);
            }
            *gen_ = gen_.wrapping_add(1);
            if *gen_ == 0 {
                marks.iter_mut().for_each(|m| *m = 0);
                *gen_ = 1;
            }
            let g = *gen_;
            let mut cand: BinaryHeap<std::cmp::Reverse<D>> = BinaryHeap::new();
            let mut res: BinaryHeap<D> = BinaryHeap::new();
            for &e in eps {
                marks[e.1 as usize] = g;
                cand.push(std::cmp::Reverse(e));
                if ok(e.1) {
                    res.push(e);
                }
            }
            let mut buf = Vec::with_capacity(64);
            while let Some(std::cmp::Reverse(c)) = cand.pop() {
                if res.len() >= ef && c.0 > res.peek().map_or(f32::MAX, |w| w.0) {
                    break;
                }
                buf.clear();
                if let Some(l) = self.node(c.1).links.get(level) {
                    buf.extend_from_slice(&l.read());
                }
                for &nb in &buf {
                    let Some(m) = marks.get_mut(nb as usize) else {
                        continue;
                    };
                    if *m == g {
                        continue;
                    }
                    *m = g;
                    let d = self.node(nb).vec.dist(q);
                    if res.len() < ef || d < res.peek().map_or(f32::MAX, |w| w.0) {
                        cand.push(std::cmp::Reverse(D(d, nb)));
                        if ok(nb) {
                            res.push(D(d, nb));
                            if res.len() > ef {
                                res.pop();
                            }
                        }
                    }
                }
            }
            res.into_sorted_vec()
        })
    }

    /// HNSW heuristic: keep a candidate only if it is closer to the base than
    /// to any already selected neighbour.
    fn select(&self, cands: &[D], m: usize) -> Vec<u32> {
        let mut out: Vec<(u32, Vec<f32>)> = Vec::with_capacity(m);
        for c in cands {
            if out.len() >= m {
                break;
            }
            let cv = self.node(c.1).vec.decode();
            if out.iter().all(|(o, _)| self.node(*o).vec.dist(&cv) > c.0) {
                out.push((c.1, cv));
            }
        }
        out.into_iter().map(|(i, _)| i).collect()
    }

    /// Runs `f` on every item: in order until the graph holds `PAGE` nodes (concurrent
    /// inserts into a near-empty graph miss each other and link poorly), then via `each`.
    fn add_many<T: Send>(&self, mut items: Vec<T>, f: impl Fn(T) + Send + Sync) {
        let warm = PAGE.saturating_sub(self.len()).min(items.len());
        let rest = items.split_off(warm);
        items.into_iter().for_each(&f);
        each(rest, f);
    }

    fn greedy(&self, q: &[f32], ep: u32, from: usize, to: usize) -> D {
        let mut best = D(self.node(ep).vec.dist(q), ep);
        for l in (to..=from).rev() {
            best = self.search_layer(q, &[best], 1, l)[0];
        }
        best
    }

    /// Inserts one vector; safe to call from many threads at once.
    pub fn insert(&self, id: u64, v: &[f32]) -> Result<()> {
        if v.len() != self.dim {
            return Err(Error(format!("dim {} != {}", v.len(), self.dim)));
        }
        self.add_raw(id, v);
        Ok(())
    }

    /// Inserts `ids.len()` vectors stored row-major in `vectors` (`ids.len() * dim` values),
    /// then [`save`](Self::save)s everything pending in one transaction and returns the pages
    /// written. Lengths are checked first: on a mismatch nothing is inserted. With the
    /// `parallel` feature the inserts run on the rayon thread pool.
    pub fn insert_batch(&self, ids: &[u64], vectors: &[f32]) -> Result<usize> {
        if ids.len().checked_mul(self.dim) != Some(vectors.len()) {
            return Err(Error(format!(
                "batch of {} ids needs {} values, got {}",
                ids.len(),
                ids.len().saturating_mul(self.dim),
                vectors.len()
            )));
        }
        let rows: Vec<(u64, &[f32])> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, &vectors[i * self.dim..(i + 1) * self.dim]))
            .collect();
        self.add_many(rows, |(id, v)| self.add_raw(id, v));
        self.save()
    }

    /// Tombstones every stored vector whose id is in `ids` and returns how many were
    /// newly tombstoned. Searches skip them at once; [`save`](Self::save) persists the
    /// tombstones, and [`compact`](Self::compact) removes the vectors. Scans every node.
    pub fn delete(&self, ids: &[u64]) -> usize {
        let ids: HashSet<u64> = ids.iter().copied().collect();
        let mut hit = Vec::new();
        for i in 0..self.len() as u32 {
            if let Some(n) = self.arena.get(i)
                && ids.contains(&n.id)
                && !n.deleted.swap(true, AO::AcqRel)
            {
                hit.push(i);
            }
        }
        self.deleted.fetch_add(hit.len(), AO::Relaxed);
        let count = hit.len();
        self.new_tombs.lock().extend(hit);
        count
    }

    /// Rebuilds the graph from the live vectors only, dropping tombstoned ones, and saves
    /// the result in one transaction that replaces every page. Returns the number of
    /// vectors removed (`0`: nothing to do, nothing written). Vectors keep their stored
    /// encoding; with the `parallel` feature they are re-inserted on rayon. Node order
    /// changes, so cost is that of a full build.
    pub fn compact(&mut self) -> Result<usize> {
        let removed = *self.deleted.get_mut();
        if removed == 0 {
            return Ok(0);
        }
        let old = std::mem::replace(&mut self.arena, Arena::new());
        *self.entry.get_mut() = u64::MAX;
        *self.deleted.get_mut() = 0;
        self.new_tombs.get_mut().clear();
        self.dirty.get_mut().clear();
        let live: Vec<Node> = old
            .into_nodes()
            .filter(|n| !n.deleted.load(AO::Relaxed))
            .collect();
        self.add_many(live, |n| {
            let q = n.vec.decode();
            self.add(n.id, &q, n.vec);
        });
        *self.rewrite.get_mut() = true;
        self.save()?;
        Ok(removed)
    }

    fn add_raw(&self, id: u64, v: &[f32]) {
        let mut q = v.to_vec();
        normalize(&mut q);
        let vec = Vector::encode(&q, self.params.quant);
        self.add(id, &q, vec);
    }

    /// Links an encoded vector into the graph; `q` is its normalized `f32` form.
    fn add(&self, id: u64, q: &[f32], vec: Vector) {
        let level = self.random_level();
        let m0 = self.params.m * 2;
        let links = (0..=level)
            .map(|l| RwLock::new(Vec::with_capacity(if l == 0 { m0 } else { self.params.m })))
            .collect();
        let me = self.arena.push(Node {
            id,
            vec,
            links,
            deleted: AtomicBool::new(false),
        });
        self.mark(me);
        let entry = {
            let e = *self.entry.read();
            if e == u64::MAX {
                let mut w = self.entry.write();
                if *w == u64::MAX {
                    *w = pack(me, level);
                    return;
                }
                *w
            } else {
                e
            }
        };
        let (ep, ep_level) = unpack(entry);
        let mut eps = vec![if ep_level > level {
            self.greedy(q, ep, ep_level, level + 1)
        } else {
            D(self.node(ep).vec.dist(q), ep)
        }];
        for l in (0..=level.min(ep_level)).rev() {
            let w = self.search_layer(q, &eps, self.params.ef_construction, l);
            let mmax = if l == 0 { m0 } else { self.params.m };
            let nbrs = self.select(&w, self.params.m);
            *self.node(me).links[l].write() = nbrs.clone();
            for nb in nbrs {
                self.link(nb, me, l, mmax);
            }
            eps = w;
        }
        if level > ep_level {
            let mut w = self.entry.write();
            if unpack(*w).1 < level {
                *w = pack(me, level);
            }
        }
    }

    fn link(&self, from: u32, to: u32, l: usize, mmax: usize) {
        let n = self.node(from);
        let mut list = n.links[l].write();
        if list.contains(&to) {
            return;
        }
        list.push(to);
        if list.len() > mmax {
            let base = n.vec.decode();
            let mut c: Vec<D> = list
                .iter()
                .map(|&i| D(self.node(i).vec.dist(&base), i))
                .collect();
            c.sort();
            *list = self.select(&c, mmax);
        }
        drop(list);
        self.mark(from);
    }

    fn mark(&self, i: u32) {
        self.dirty.lock().insert(i / PAGE as u32);
    }

    /// Top-`k` by cosine similarity: `(id, score)` with higher score first.
    pub fn search(&self, v: &[f32], k: usize) -> Vec<(u64, f32)> {
        self.search_inner(v, k, None)
    }

    /// Like [`search`](Self::search), but only ids for which `filter` returns `true` are
    /// returned. Rejected nodes are still walked through, so the graph stays connected; an
    /// allow-list is `|id| allowed.contains(&id)`. A very selective filter makes the walk
    /// visit much of the graph before it finds `k` matches.
    pub fn search_filtered(
        &self,
        v: &[f32],
        k: usize,
        filter: impl Fn(u64) -> bool,
    ) -> Vec<(u64, f32)> {
        self.search_inner(v, k, Some(&filter))
    }

    fn search_inner(
        &self,
        v: &[f32],
        k: usize,
        filter: Option<&dyn Fn(u64) -> bool>,
    ) -> Vec<(u64, f32)> {
        let e = *self.entry.read();
        if e == u64::MAX {
            return Vec::new();
        }
        let mut q = v.to_vec();
        normalize(&mut q);
        let (ep, top) = unpack(e);
        let best = if top == 0 {
            D(self.node(ep).vec.dist(&q), ep)
        } else {
            self.greedy(&q, ep, top, 1)
        };
        let rerank = self.params.quant == Quant::I8Rerank;
        let want = if rerank {
            k.saturating_mul(self.rerank.load(AO::Relaxed).max(1))
        } else {
            k
        };
        let ef = self.ef_search.load(AO::Relaxed).max(want);
        let tombs = self.deleted.load(AO::Relaxed) > 0;
        let accept = |i: u32| {
            let n = self.node(i);
            !(tombs && n.deleted.load(AO::Relaxed)) && filter.is_none_or(|f| f(n.id))
        };
        let accept: Option<&dyn Fn(u32) -> bool> = (tombs || filter.is_some()).then_some(&accept);
        let mut r = self.search_layer_if(&q, &[best], ef, 0, accept);
        r.truncate(want);
        if rerank {
            for d in &mut r {
                d.0 = self.node(d.1).vec.exact_dist(&q);
            }
            r.sort();
        }
        r.truncate(k);
        r.into_iter()
            .map(|d| (self.node(d.1).id, 1.0 - d.0))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vecs(n: usize, dim: usize) -> Vec<Vec<f32>> {
        let mut s = 1u64;
        (0..n)
            .map(|_| {
                (0..dim)
                    .map(|_| {
                        s ^= s << 13;
                        s ^= s >> 7;
                        s ^= s << 17;
                        (s % 2000) as f32 / 1000.0 - 1.0
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn finds_itself_and_persists() {
        let dir = std::env::temp_dir().join(format!("vecdb-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let data = vecs(2000, 32);
        for quant in [Quant::F32, Quant::I8] {
            let _ = std::fs::remove_file(&path);
            let db = VecDb::open(
                path.to_str().unwrap(),
                32,
                Params {
                    quant,
                    ..Params::default()
                },
            )
            .unwrap();
            std::thread::scope(|s| {
                for chunk in data.chunks(500).enumerate() {
                    let db = &db;
                    s.spawn(move || {
                        for (j, v) in chunk.1.iter().enumerate() {
                            db.insert((chunk.0 * 500 + j) as u64, v).unwrap();
                        }
                    });
                }
            });
            db.save().unwrap();
            drop(db);
            let db = VecDb::open(path.to_str().unwrap(), 32, Params::default()).unwrap();
            assert_eq!(db.len(), 2000);
            let hits = (0..200)
                .filter(|&i| db.search(&data[i], 1)[0].0 == i as u64)
                .count();
            assert!(hits >= 190, "{quant:?}: {hits}/200");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
