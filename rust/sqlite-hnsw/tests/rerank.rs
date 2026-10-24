//! `Quant::I8Rerank`: `i8` graph walk, `f32` rescoring of the best `k * rerank` candidates.

mod common;

use sqlite_hnsw::{Params, Quant, VecDb};

const DIM: usize = 32;

fn build(dir: &tempfile::TempDir, quant: Quant, data: &[Vec<f32>]) -> VecDb {
    let db = common::open(
        dir,
        DIM,
        Params {
            quant,
            ef_search: 32,
            ..Params::default()
        },
    );
    for (i, v) in data.iter().enumerate() {
        db.insert(i as u64, v).unwrap();
    }
    db
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / (n(a) * n(b))
}

fn mean_recall(db: &VecDb, data: &[Vec<f32>], queries: &[Vec<f32>]) -> f64 {
    queries
        .iter()
        .map(|q| {
            let got: Vec<u64> = db.search(q, 10).into_iter().map(|h| h.0).collect();
            common::recall(&got, &common::exact(data, q, 10, |_| true))
        })
        .sum::<f64>()
        / queries.len() as f64
}

#[test]
fn rerank_scores_are_exact_f32_cosine() {
    let data = common::vecs(1500, DIM, 21);
    let queries = common::vecs(20, DIM, 99);
    let dir = tempfile::tempdir().unwrap();
    let db = build(&dir, Quant::I8Rerank, &data);
    for q in &queries {
        let hits = db.search(q, 10);
        assert_eq!(hits.len(), 10);
        assert!(hits.windows(2).all(|w| w[0].1 >= w[1].1));
        for (id, score) in hits {
            let want = cosine(q, &data[id as usize]);
            assert!((score - want).abs() < 1e-5, "{id}: {score} vs {want}");
        }
    }
}

#[test]
fn rerank_recall_is_at_least_plain_i8() {
    let data = common::vecs(2000, DIM, 5);
    let queries = common::vecs(50, DIM, 77);
    let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let plain = mean_recall(&build(&d1, Quant::I8, &data), &data, &queries);
    let reranked = mean_recall(&build(&d2, Quant::I8Rerank, &data), &data, &queries);
    assert!(reranked >= plain, "rerank {reranked} < i8 {plain}");
    assert!(reranked >= 0.9, "rerank recall {reranked}");
}

#[test]
fn rerank_factor_can_change_at_runtime() {
    let data = common::vecs(1000, DIM, 8);
    let dir = tempfile::tempdir().unwrap();
    let db = build(&dir, Quant::I8Rerank, &data);
    for factor in [0, 1, 16] {
        db.set_rerank(factor);
        let hits = db.search(&data[3], 5);
        assert_eq!(hits.len(), 5, "factor {factor}");
        assert_eq!(hits[0].0, 3, "factor {factor}");
    }
}

#[test]
fn rerank_vectors_survive_reopen() {
    let data = common::vecs(1200, DIM, 13);
    let dir = tempfile::tempdir().unwrap();
    {
        let db = build(&dir, Quant::I8Rerank, &data);
        db.save().unwrap();
    }
    // The stored mode wins over the one passed on reopen.
    let db = common::open(&dir, DIM, Params::default());
    assert!(format!("{db:?}").contains("I8Rerank"));
    assert_eq!(db.len(), 1200);
    let hits = db.search(&data[7], 3);
    assert_eq!(hits[0].0, 7);
    assert!((hits[0].1 - 1.0).abs() < 1e-5, "{}", hits[0].1);
}
