//! `insert_batch`: validated up front, inserted (on rayon with `parallel`), saved in one transaction.

mod common;

use sqlite_hnsw::{Params, Quant};

const DIM: usize = 16;

fn flat(data: &[Vec<f32>]) -> Vec<f32> {
    data.concat()
}

#[test]
fn batch_inserts_saves_and_reopens() {
    let data = common::vecs(3000, DIM, 31);
    let ids: Vec<u64> = (0..3000).map(|i| 10_000 + i).collect();
    let dir = tempfile::tempdir().unwrap();
    {
        let db = common::open(&dir, DIM, Params::default());
        let pages = db.insert_batch(&ids, &flat(&data)).unwrap();
        assert_eq!(pages, 3, "3000 nodes fill three 1024-node pages");
        assert_eq!(db.len(), 3000);
        assert_eq!(db.save().unwrap(), 0, "the batch already saved everything");
    }
    let db = common::open(&dir, DIM, Params::default());
    assert_eq!(db.len(), 3000);
    let hits = (0..200)
        .filter(|&i| db.search(&data[i], 1)[0].0 == ids[i])
        .count();
    assert!(hits >= 190, "{hits}/200");
}

#[test]
fn batch_appends_to_an_existing_index() {
    let data = common::vecs(1200, DIM, 4);
    let dir = tempfile::tempdir().unwrap();
    let db = common::open(
        &dir,
        DIM,
        Params {
            quant: Quant::I8,
            ..Params::default()
        },
    );
    for (i, v) in data[..200].iter().enumerate() {
        db.insert(i as u64, v).unwrap();
    }
    let ids: Vec<u64> = (200..1200).collect();
    db.insert_batch(&ids, &flat(&data[200..])).unwrap();
    assert_eq!(db.len(), 1200);
    let hits = (0..1200)
        .step_by(10)
        .filter(|&i| db.search(&data[i], 1)[0].0 == i as u64)
        .count();
    assert!(hits >= 114, "{hits}/120");
}

#[test]
fn length_mismatch_inserts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = common::open(&dir, DIM, Params::default());
    let data = flat(&common::vecs(3, DIM, 1));
    let err = db.insert_batch(&[1, 2], &data).unwrap_err();
    assert!(err.to_string().contains("batch"), "{err}");
    assert!(db.insert_batch(&[1, 2, 3], &data[1..]).is_err());
    assert!(db.is_empty());
}

#[test]
fn empty_batch_is_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let db = common::open(&dir, DIM, Params::default());
    assert_eq!(db.insert_batch(&[], &[]).unwrap(), 0);
    db.insert(1, &[0.5; DIM]).unwrap();
    assert_eq!(
        db.insert_batch(&[], &[]).unwrap(),
        1,
        "pending insert saved"
    );
}
