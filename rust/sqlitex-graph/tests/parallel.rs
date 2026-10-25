//! `parallel` feature: levels expanded on rayon's pool give exactly the rows, in exactly
//! the order, of the walk on the calling thread.
#![cfg(feature = "parallel")]
mod common;
use common::{Rng, random_graph};
use sqlitex_graph::graph::DEFAULT_PARALLEL_MIN_FRONTIER;

#[test]
fn parallel_levels_equal_sequential_levels() {
    // Wide: a few hundred start names put level 1 above any small threshold.
    let g = random_graph(17, 3_000, 400, 40_000);
    assert_eq!(g.parallel_min_frontier(), DEFAULT_PARALLEL_MIN_FRONTIER);
    let mut rng = Rng(77);
    for _ in 0..8 {
        let names: Vec<String> = (0..300)
            .map(|_| format!("n{}", 1 + rng.below(3_000)))
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let depth = 1 + rng.below(4);
        g.set_parallel_min_frontier(usize::MAX);
        let sequential = g.impact_many(1, &refs, depth);
        g.set_parallel_min_frontier(1);
        let parallel = g.impact_many(1, &refs, depth);
        assert!(!sequential.is_empty());
        assert_eq!(parallel, sequential);
        // Single-source walks go parallel once their frontier grows.
        let one = refs[0];
        g.set_parallel_min_frontier(usize::MAX);
        let sequential = g.impact(1, one, depth);
        g.set_parallel_min_frontier(1);
        assert_eq!(g.impact(1, one, depth), sequential);
    }
    g.set_parallel_min_frontier(DEFAULT_PARALLEL_MIN_FRONTIER);
}
