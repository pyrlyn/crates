//! The entry point, node levels and links are persisted: a reopened index walks
//! the same graph and returns the same hits, bit for bit.

mod common;

use sqlitex_vector::{Params, Quant};

const DIM: usize = 24;

#[test]
fn reopened_index_returns_identical_hits() {
    // 2500 nodes span three pages and give several upper levels at m = 8.
    let data = common::vecs(2500, DIM, 71);
    let queries = common::vecs(30, DIM, 72);
    for quant in [Quant::F32, Quant::I8, Quant::I8Rerank] {
        let dir = tempfile::tempdir().unwrap();
        let params = Params {
            m: 8,
            ef_search: 40,
            quant,
            ..Params::default()
        };
        let before: Vec<_> = {
            let db = common::open(&dir, DIM, params);
            for (i, v) in data.iter().enumerate() {
                db.insert(i as u64, v).unwrap();
            }
            db.save().unwrap();
            queries.iter().map(|q| db.search(q, 10)).collect()
        };
        let db = common::open(&dir, DIM, params);
        assert_eq!(db.len(), data.len(), "{quant:?}");
        let after: Vec<_> = queries.iter().map(|q| db.search(q, 10)).collect();
        assert_eq!(before, after, "{quant:?}");
        assert_eq!(
            db.save().unwrap(),
            0,
            "{quant:?}: a fresh open has nothing dirty"
        );
    }
}

#[test]
fn inserts_after_reopen_extend_the_stored_graph() {
    let data = common::vecs(1500, DIM, 73);
    let dir = tempfile::tempdir().unwrap();
    {
        let db = common::open(&dir, DIM, Params::default());
        for (i, v) in data[..1000].iter().enumerate() {
            db.insert(i as u64, v).unwrap();
        }
        db.save().unwrap();
    }
    {
        let db = common::open(&dir, DIM, Params::default());
        for (i, v) in data.iter().enumerate().skip(1000) {
            db.insert(i as u64, v).unwrap();
        }
        db.save().unwrap();
    }
    let db = common::open(&dir, DIM, Params::default());
    assert_eq!(db.len(), 1500);
    let found = (0..1500)
        .step_by(10)
        .filter(|&i| db.search(&data[i], 1)[0].0 == i as u64)
        .count();
    assert!(found >= 147, "{found}/150");
}
