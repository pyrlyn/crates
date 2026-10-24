//! Helpers shared by the integration tests.
#![allow(dead_code)]

use sqlite_hnsw::{Params, VecDb};

/// Deterministic pseudo-random vectors with components in `[-1, 1)`.
pub fn vecs(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut s = seed;
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

/// Opens `v.db` inside `dir`.
pub fn open(dir: &tempfile::TempDir, dim: usize, params: Params) -> VecDb {
    VecDb::open(dir.path().join("v.db").to_str().unwrap(), dim, params).unwrap()
}

/// Exact top-`k` ids by cosine similarity among `data[i]` with `keep(i)`.
pub fn exact(data: &[Vec<f32>], q: &[f32], k: usize, keep: impl Fn(usize) -> bool) -> Vec<u64> {
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let qn = norm(q);
    let mut all: Vec<(f32, u64)> = data
        .iter()
        .enumerate()
        .filter(|(i, _)| keep(*i))
        .map(|(i, v)| {
            let d: f32 = v.iter().zip(q).map(|(a, b)| a * b).sum();
            (d / (norm(v) * qn), i as u64)
        })
        .collect();
    all.sort_by(|a, b| b.0.total_cmp(&a.0));
    all.into_iter().take(k).map(|x| x.1).collect()
}

/// Fraction of `want` found in `got`.
pub fn recall(got: &[u64], want: &[u64]) -> f64 {
    want.iter().filter(|w| got.contains(w)).count() as f64 / want.len().max(1) as f64
}
