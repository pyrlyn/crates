//! `Params::for_recall`: documented values, ordering, and a working index.

mod common;

use sqlite_hnsw::{Params, Quant, Recall};

const QUANTS: [Quant; 3] = [Quant::F32, Quant::I8, Quant::I8Rerank];

#[test]
fn presets_match_the_documented_table() {
    for quant in QUANTS {
        for target in [Recall::P90, Recall::P95] {
            let p = Params::for_recall(target, quant);
            assert_eq!((p.m, p.ef_construction, p.quant), (32, 400, quant));
            let want = match (target, quant) {
                (Recall::P90, _) => 768,
                (Recall::P95, Quant::I8) => 1280,
                (Recall::P95, _) => 1024,
            };
            assert_eq!(p.ef_search, want, "{target:?} {quant:?}");
        }
    }
}

#[test]
fn higher_target_never_searches_less() {
    for quant in QUANTS {
        let (p90, p95) = (
            Params::for_recall(Recall::P90, quant),
            Params::for_recall(Recall::P95, quant),
        );
        assert!(p95.ef_search >= p90.ef_search, "{quant:?}");
    }
}

/// Sanity check on small data (the presets are sized for 100k vectors, so here
/// recall should be well above the target).
#[test]
fn presets_build_an_index_that_meets_its_target() {
    const DIM: usize = 16;
    let data = common::vecs(1200, DIM, 61);
    let queries = common::vecs(20, DIM, 62);
    let ids: Vec<u64> = (0..1200).collect();
    for quant in [Quant::F32, Quant::I8] {
        let dir = tempfile::tempdir().unwrap();
        let db = common::open(&dir, DIM, Params::for_recall(Recall::P95, quant));
        db.insert_batch(&ids, &data.concat()).unwrap();
        for (target, min) in [(Recall::P90, 0.90), (Recall::P95, 0.95)] {
            db.set_ef_search(Params::for_recall(target, quant).ef_search);
            let recall = queries
                .iter()
                .map(|q| {
                    let got: Vec<u64> = db.search(q, 10).into_iter().map(|h| h.0).collect();
                    common::recall(&got, &common::exact(&data, q, 10, |_| true))
                })
                .sum::<f64>()
                / queries.len() as f64;
            assert!(recall >= min, "{quant:?} {target:?}: {recall}");
        }
    }
}
