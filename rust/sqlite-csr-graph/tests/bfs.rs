//! The pooled-scratch, level-by-level traversals against the plain HashMap BFS they
//! replaced, on random in-memory graphs (base + delta, no SQLite): `impact`,
//! multi-source `impact_many`, `neighbors`, repeated and interleaved walks on one thread.
mod common;
use common::{Rng, impact_text, random_graph};
use sqlite_csr_graph::graph::{Graph, Snapshot};
use sqlite_csr_graph::segment::{KIND_IMPORT, KIND_REF};
use std::collections::{HashMap, HashSet};

/// The `impact` BFS as it was before the scratch pool, with several start names.
fn reference_impact(g: &Graph, names: &[&str], depth: u32) -> Vec<(u32, String, String)> {
    let dict = g.dict.load();
    let snap: std::sync::Arc<Snapshot> = g.snapshot(1).unwrap();
    let mut expanded: HashSet<u32> = HashSet::new();
    let mut frontier = vec![];
    for n in names {
        if let Some(&id) = dict.name_ids.get(*n)
            && id != 0
            && expanded.insert(id)
        {
            frontier.push(id);
        }
    }
    let mut seen: HashMap<(u32, u32), u32> = HashMap::new();
    let mut level = 1;
    while !frontier.is_empty() && level <= depth {
        let mut next = Vec::new();
        for &tip in &frontier {
            let mut add = |file: u32, owner: u32, next: &mut Vec<u32>| {
                seen.entry((file, owner)).or_insert(level);
                if owner != 0 && expanded.insert(owner) {
                    next.push(owner);
                }
            };
            snap.users(tip, |e| {
                if e.kind == KIND_IMPORT {
                    snap.filedefs(e.file, |d| add(e.file, d, &mut next));
                } else {
                    add(e.file, e.other, &mut next);
                }
            });
        }
        frontier = next;
        level += 1;
    }
    let mut rows: Vec<(u32, String, String)> = seen
        .into_iter()
        .map(|((f, o), d)| (d, dict.path(f).to_string(), dict.name(o).to_string()))
        .collect();
    rows.sort();
    rows
}

/// The `neighbors` BFS as it was before the scratch pool.
fn reference_neighbors(g: &Graph, name: &str, depth: u32, outgoing: bool) -> Vec<(u32, String)> {
    let dict = g.dict.load();
    let snap = g.snapshot(1).unwrap();
    let Some(&start) = dict.name_ids.get(name) else {
        return vec![];
    };
    let mut level_of: HashMap<u32, u32> = HashMap::from([(start, 0)]);
    let mut frontier = vec![start];
    for level in 1..=depth {
        let mut next = Vec::new();
        for &n in &frontier {
            let mut push = |m: u32| {
                if m != 0 && !level_of.contains_key(&m) {
                    level_of.insert(m, level);
                    next.push(m);
                }
            };
            if outgoing {
                snap.callees(n, |e| push(e.other));
            } else {
                snap.users(n, |e| {
                    if e.kind == KIND_REF {
                        push(e.other)
                    } else {
                        snap.filedefs(e.file, &mut push)
                    }
                });
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    let mut v: Vec<(u32, String)> = level_of
        .into_iter()
        .filter(|(n, _)| *n != start)
        .map(|(n, l)| (l, dict.name(n).to_string()))
        .collect();
    v.sort();
    v
}

fn neighbors_text(g: &Graph, rows: &[(u32, u32)]) -> Vec<(u32, String)> {
    let d = g.dict.load();
    rows.iter()
        .map(|(l, n)| (*l, d.name(*n).to_string()))
        .collect()
}

#[test]
fn impact_matches_the_reference_bfs_over_many_reused_walks() {
    for seed in 1..=6 {
        let g = random_graph(seed, 300, 60, 2_000);
        let mut rng = Rng(seed * 31);
        for _ in 0..60 {
            let name = format!("n{}", 1 + rng.below(300));
            let depth = rng.below(6);
            let got = impact_text(&g, &g.impact(1, &name, depth));
            // The same rows as the reference, and in (depth, path, scope) order.
            let mut sorted = got.clone();
            sorted.sort();
            assert_eq!(
                sorted,
                reference_impact(&g, &[&name], depth),
                "{name} d{depth}"
            );
            assert!(
                got.windows(2).all(|w| w[0] <= w[1]),
                "{name}: rows not in (depth, path, scope) order"
            );
        }
    }
}

#[test]
fn impact_many_is_one_bfs_from_every_start() {
    let g = random_graph(11, 400, 80, 3_000);
    let mut rng = Rng(5);
    for _ in 0..40 {
        let names: Vec<String> = (0..1 + rng.below(5))
            .map(|_| format!("n{}", 1 + rng.below(400)))
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let depth = 1 + rng.below(4);
        let mut got = impact_text(&g, &g.impact_many(1, &refs, depth));
        got.sort();
        assert_eq!(got, reference_impact(&g, &refs, depth), "{names:?}");
        // Every row's depth is its smallest depth from any single start.
        let mut best: HashMap<(String, String), u32> = HashMap::new();
        for n in &refs {
            for (d, p, s) in impact_text(&g, &g.impact(1, n, depth)) {
                let e = best.entry((p, s)).or_insert(d);
                *e = (*e).min(d);
            }
        }
        assert_eq!(got.len(), best.len(), "{names:?}: row sets differ");
        for (d, p, s) in got {
            assert_eq!(best[&(p, s)], d);
        }
    }
    assert!(g.impact_many(1, &[], 3).is_empty());
    assert!(g.impact_many(1, &["no-such-name", ""], 3).is_empty());
}

#[test]
fn neighbors_match_the_reference_bfs() {
    let g = random_graph(3, 250, 50, 1_500);
    let mut rng = Rng(9);
    for _ in 0..80 {
        let name = format!("n{}", 1 + rng.below(250));
        let depth = rng.below(5);
        for outgoing in [false, true] {
            let got = neighbors_text(&g, &g.neighbors(1, &name, depth, outgoing));
            let mut sorted = got.clone();
            sorted.sort();
            assert_eq!(sorted, reference_neighbors(&g, &name, depth, outgoing));
            assert!(got.windows(2).all(|w| w[0] <= w[1]));
        }
    }
}

#[test]
fn interleaved_walks_on_one_thread_do_not_share_state() {
    let g = random_graph(21, 300, 60, 2_500);
    let (a, b) = ("n7", "n8");
    let want_a = g.impact(1, a, 5);
    let want_b = g.impact(1, b, 5);
    let mut wa = g.impact_walk(1, &[a], 5).unwrap();
    let mut wb = g.impact_walk(1, &[b], 5).unwrap();
    let (mut got_a, mut got_b) = (vec![], vec![]);
    loop {
        let la = wa.next_level();
        let lb = wb.next_level();
        if la.is_none() && lb.is_none() {
            break;
        }
        got_a.extend(la.into_iter().flatten());
        got_b.extend(lb.into_iter().flatten());
    }
    assert_eq!(got_a, want_a);
    assert_eq!(got_b, want_b);
}

#[test]
fn a_walk_expands_one_level_per_call() {
    let g = random_graph(2, 200, 40, 3_000);
    let mut w = g.impact_walk(1, &["n1", "n2", "n3"], 10).unwrap();
    assert_eq!(w.level(), 0);
    let first = w.next_level().expect("dense graph: level 1 has rows");
    assert_eq!(w.level(), 1);
    assert!(first.iter().all(|r| r.depth == 1));
    assert!(g.impact_walk(99, &["n1"], 3).is_none(), "unknown root");
}
