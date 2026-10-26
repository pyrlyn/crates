//! Public-API integration tests: open, insert, search, save, reopen.

use sqlitex_vector::{Params, Quant, VecDb};

fn vecs(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
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

fn open(dir: &tempfile::TempDir, dim: usize, params: Params) -> VecDb {
    VecDb::open(dir.path().join("v.db").to_str().unwrap(), dim, params).unwrap()
}

#[test]
fn empty_index_returns_no_hits() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, 8, Params::default());
    assert!(db.is_empty());
    assert!(db.search(&[1.0; 8], 5).is_empty());
}

#[test]
fn search_scores_are_cosine_and_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, 16, Params::default());
    for (i, v) in vecs(300, 16, 7).iter().enumerate() {
        db.insert(i as u64, v).unwrap();
    }
    let q = vecs(300, 16, 7)[42].clone();
    let hits = db.search(&q, 10);
    assert_eq!(hits.len(), 10);
    assert_eq!(hits[0].0, 42);
    assert!(
        (hits[0].1 - 1.0).abs() < 1e-4,
        "self-similarity {}",
        hits[0].1
    );
    assert!(hits.windows(2).all(|w| w[0].1 >= w[1].1));
}

#[test]
fn reopen_keeps_stored_params_and_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let data = vecs(1500, 24, 3);
    {
        let db = open(
            &dir,
            24,
            Params {
                m: 8,
                quant: Quant::I8,
                ..Params::default()
            },
        );
        for (i, v) in data.iter().enumerate() {
            db.insert(i as u64, v).unwrap();
        }
        assert!(db.save().unwrap() > 0);
        assert_eq!(db.save().unwrap(), 0, "nothing dirty after a save");
    }
    // Different m/quant on reopen: the stored ones win.
    let db = open(
        &dir,
        24,
        Params {
            m: 32,
            quant: Quant::F32,
            ef_search: 128,
            ..Params::default()
        },
    );
    assert_eq!(db.len(), 1500);
    let hits = (0..100)
        .filter(|&i| db.search(&data[i], 1)[0].0 == i as u64)
        .count();
    assert!(hits >= 95, "{hits}/100");
    let dbg = format!("{db:?}");
    assert!(dbg.contains("I8") && dbg.contains("m: 8"), "{dbg}");
}

#[test]
fn reopen_with_other_dimension_fails() {
    let dir = tempfile::tempdir().unwrap();
    {
        let db = open(&dir, 4, Params::default());
        db.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        db.save().unwrap();
    }
    let err = VecDb::open(
        dir.path().join("v.db").to_str().unwrap(),
        8,
        Params::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("dim"), "{err}");
}

#[test]
fn concurrent_inserts_and_searches() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, 16, Params::default());
    let data = vecs(2000, 16, 11);
    std::thread::scope(|s| {
        for (c, chunk) in data.chunks(500).enumerate() {
            let db = &db;
            s.spawn(move || {
                for (j, v) in chunk.iter().enumerate() {
                    db.insert((c * 500 + j) as u64, v).unwrap();
                    let _ = db.search(v, 3);
                }
            });
        }
    });
    assert_eq!(db.len(), 2000);
}
