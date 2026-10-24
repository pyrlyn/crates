//! `search_filtered`: predicate and allow-list search.

mod common;

use std::collections::HashSet;

use sqlite_hnsw::{Params, VecDb};

const DIM: usize = 24;

fn build(dir: &tempfile::TempDir, data: &[Vec<f32>]) -> VecDb {
    let db = common::open(dir, DIM, Params::default());
    for (i, v) in data.iter().enumerate() {
        db.insert(i as u64, v).unwrap();
    }
    db
}

#[test]
fn predicate_keeps_only_matching_ids_with_good_recall() {
    let data = common::vecs(2000, DIM, 17);
    let queries = common::vecs(30, DIM, 404);
    let dir = tempfile::tempdir().unwrap();
    let db = build(&dir, &data);
    let mut total = 0.0;
    for q in &queries {
        let hits = db.search_filtered(q, 10, |id| id % 3 == 0);
        assert_eq!(hits.len(), 10);
        assert!(hits.iter().all(|h| h.0 % 3 == 0), "{hits:?}");
        assert!(hits.windows(2).all(|w| w[0].1 >= w[1].1));
        let got: Vec<u64> = hits.iter().map(|h| h.0).collect();
        total += common::recall(&got, &common::exact(&data, q, 10, |i| i % 3 == 0));
    }
    let recall = total / queries.len() as f64;
    assert!(recall >= 0.9, "filtered recall {recall}");
}

#[test]
fn small_allow_list_is_found() {
    let data = common::vecs(2000, DIM, 23);
    let dir = tempfile::tempdir().unwrap();
    let db = build(&dir, &data);
    let allowed: HashSet<u64> = (0..2000).step_by(97).collect();
    let q = &data[500];
    let hits = db.search_filtered(q, 10, |id| allowed.contains(&id));
    assert_eq!(hits.len(), 10);
    assert!(hits.iter().all(|h| allowed.contains(&h.0)));
    let got: Vec<u64> = hits.iter().map(|h| h.0).collect();
    let want = common::exact(&data, q, 10, |i| allowed.contains(&(i as u64)));
    assert!(common::recall(&got, &want) >= 0.9, "{got:?} vs {want:?}");
}

#[test]
fn filter_with_fewer_matches_than_k_returns_them_all() {
    let data = common::vecs(800, DIM, 2);
    let dir = tempfile::tempdir().unwrap();
    let db = build(&dir, &data);
    let hits = db.search_filtered(&data[0], 10, |id| id == 5 || id == 700);
    let mut ids: Vec<u64> = hits.iter().map(|h| h.0).collect();
    ids.sort_unstable();
    assert_eq!(ids, [5, 700]);
    assert!(db.search_filtered(&data[0], 10, |_| false).is_empty());
}

#[test]
fn accept_all_filter_matches_plain_search() {
    let data = common::vecs(1000, DIM, 9);
    let dir = tempfile::tempdir().unwrap();
    let db = build(&dir, &data);
    for q in &data[..20] {
        assert_eq!(db.search_filtered(q, 5, |_| true), db.search(q, 5));
    }
}
