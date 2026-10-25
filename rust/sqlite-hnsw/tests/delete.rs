//! `delete` (tombstones) and `compact`.

mod common;

use sqlite_hnsw::{Params, Quant, VecDb};

const DIM: usize = 16;

fn build(dir: &tempfile::TempDir, quant: Quant, data: &[Vec<f32>]) -> VecDb {
    let db = common::open(
        dir,
        DIM,
        Params {
            quant,
            ..Params::default()
        },
    );
    for (i, v) in data.iter().enumerate() {
        db.insert(i as u64, v).unwrap();
    }
    db
}

fn self_hits(db: &VecDb, data: &[Vec<f32>], ids: impl Iterator<Item = usize>) -> (usize, usize) {
    let ids: Vec<usize> = ids.collect();
    let hits = ids
        .iter()
        .filter(|&&i| db.search(&data[i], 1).first().map(|h| h.0) == Some(i as u64))
        .count();
    (hits, ids.len())
}

#[test]
fn deleted_ids_disappear_from_search_at_once() {
    let data = common::vecs(1500, DIM, 3);
    let dir = tempfile::tempdir().unwrap();
    let db = build(&dir, Quant::F32, &data);
    let gone: Vec<u64> = (0..1500).step_by(2).collect();
    assert_eq!(db.delete(&gone), 750);
    assert_eq!(db.delete(&gone), 0, "already tombstoned");
    assert_eq!(db.delete(&[99_999]), 0, "unknown id");
    assert_eq!(db.tombstones(), 750);
    assert_eq!(db.len(), 1500, "tombstones stay until compact");
    for q in data.iter().take(100) {
        let hits = db.search(q, 10);
        assert_eq!(hits.len(), 10);
        assert!(hits.iter().all(|h| h.0 % 2 == 1), "{hits:?}");
    }
    let (hits, n) = self_hits(&db, &data, (1..1500).step_by(10));
    assert!(hits * 100 >= n * 95, "{hits}/{n}");
}

#[test]
fn delete_hits_every_copy_of_a_duplicate_id() {
    let dir = tempfile::tempdir().unwrap();
    let db = common::open(&dir, DIM, Params::default());
    db.insert(7, &[1.0; DIM]).unwrap();
    db.insert(7, &[0.5; DIM]).unwrap();
    db.insert(8, &[0.25; DIM]).unwrap();
    assert_eq!(db.delete(&[7]), 2);
    let hits = db.search(&[1.0; DIM], 5);
    assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), [8]);
}

#[test]
fn tombstones_persist_only_after_save() {
    let data = common::vecs(1100, DIM, 5);
    let dir = tempfile::tempdir().unwrap();
    {
        let db = build(&dir, Quant::I8, &data);
        db.save().unwrap();
        db.delete(&[1, 2, 3]);
    }
    let db = common::open(&dir, DIM, Params::default());
    assert_eq!(db.tombstones(), 0, "unsaved delete is lost");
    assert_eq!(db.delete(&[1, 2, 3, 1050]), 4);
    assert_eq!(db.save().unwrap(), 0, "tombstones need no page rewrite");
    drop(db);
    let db = common::open(&dir, DIM, Params::default());
    assert_eq!(db.tombstones(), 4);
    for id in [1u64, 2, 3, 1050] {
        assert!(
            db.search(&data[id as usize], 10).iter().all(|h| h.0 != id),
            "{id} came back"
        );
    }
}

#[test]
fn compact_drops_tombstones_and_survives_reopen() {
    let data = common::vecs(2500, DIM, 41);
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = build(&dir, Quant::F32, &data);
        db.save().unwrap();
        assert_eq!(db.compact().unwrap(), 0, "nothing to compact");
        let gone: Vec<u64> = (0..2500).filter(|i| i % 5 != 0).collect();
        assert_eq!(db.delete(&gone), 2000);
        assert_eq!(db.compact().unwrap(), 2000);
        assert_eq!((db.len(), db.tombstones()), (500, 0));
        assert_eq!(db.save().unwrap(), 0, "compact saved everything");
    }
    let db = common::open(&dir, DIM, Params::default());
    assert_eq!((db.len(), db.tombstones()), (500, 0));
    let (hits, n) = self_hits(&db, &data, (0..2500).step_by(5));
    assert!(hits * 100 >= n * 95, "{hits}/{n}");
    for q in data.iter().take(50) {
        assert!(db.search(q, 10).iter().all(|h| h.0 % 5 == 0));
    }
}

#[test]
fn compact_everything_leaves_a_usable_empty_index() {
    let data = common::vecs(300, DIM, 8);
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = build(&dir, Quant::F32, &data);
        let all: Vec<u64> = (0..300).collect();
        db.delete(&all);
        assert_eq!(db.compact().unwrap(), 300);
        assert!(db.is_empty());
        assert!(db.search(&data[0], 3).is_empty());
    }
    let db = common::open(&dir, DIM, Params::default());
    assert!(db.is_empty());
    db.insert(42, &data[0]).unwrap();
    assert_eq!(db.search(&data[0], 1)[0].0, 42);
}

#[test]
fn compact_keeps_rerank_vectors_exact() {
    let data = common::vecs(1200, DIM, 19);
    let dir = tempfile::tempdir().unwrap();
    let mut db = build(&dir, Quant::I8Rerank, &data);
    db.delete(&(0..600).collect::<Vec<u64>>());
    db.compact().unwrap();
    let hits = db.search(&data[900], 1);
    assert_eq!(hits[0].0, 900);
    assert!((hits[0].1 - 1.0).abs() < 1e-5, "{}", hits[0].1);
}

#[test]
fn filter_and_tombstones_combine() {
    let data = common::vecs(1000, DIM, 27);
    let dir = tempfile::tempdir().unwrap();
    let db = build(&dir, Quant::F32, &data);
    db.delete(&(0..1000).filter(|i| i % 4 == 0).collect::<Vec<u64>>());
    let hits = db.search_filtered(&data[0], 10, |id| id % 2 == 0);
    assert_eq!(hits.len(), 10);
    assert!(hits.iter().all(|h| h.0 % 4 == 2), "{hits:?}");
}
