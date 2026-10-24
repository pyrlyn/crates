//! N readers + 1 writer + background merges on a synthetic store; segment merge correctness.
mod common;
use sqlite_csr_graph::{DefRow, FileSymbols, RefRow, Store};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const R: &str = "/r";

/// File `i` version `v`: defines f{i}, and calls f{(i+1)%n} and marker v{v}.
fn file(i: u32, n: u32, v: u32) -> FileSymbols {
    FileSymbols {
        root: R.into(),
        path: format!("src/f{i}.rs"),
        sha: vec![v as u8],
        defs: vec![DefRow {
            name: format!("f{i}"),
            kind: "function".into(),
            line: 1,
            end_line: 9,
            ..Default::default()
        }],
        refs: vec![
            RefRow {
                name: format!("f{}", (i + 1) % n),
                scope: format!("f{i}"),
                kind: "call".into(),
                line: 2,
                ..Default::default()
            },
            RefRow {
                name: format!("v{v}"),
                scope: format!("f{i}"),
                kind: "call".into(),
                line: 3,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn readers_and_writer_and_merge() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let n = 64u32;
    let mut store = Store::open(&path).unwrap();
    for i in 0..n {
        store.upsert_file(&file(i, n, 0)).unwrap();
    }
    store.merge_now(1).unwrap();
    if let Some(g) = Arc::get_mut(&mut store.graph) {
        g.merge_threshold = 300;
    }
    let store = Arc::new(store);
    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicU64::new(0));
    let mut hs = vec![];
    for t in 0..8 {
        let (s, stop, reads) = (store.clone(), stop.clone(), reads.clone());
        hs.push(std::thread::spawn(move || {
            let conn = s.reader().unwrap();
            let mut k = t;
            while !stop.load(Ordering::Relaxed) {
                k = (k + 7) % n;
                // invariant: every file is always fully present (atomic per-file replacement)
                let callees: Vec<String> = conn
                    .prepare_cached("SELECT callee FROM graph_callees(?1, ?2) ORDER BY callee")
                    .unwrap()
                    .query_map(sqlite_csr_graph::params![format!("f{k}"), R], |r| r.get(0))
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect();
                assert_eq!(callees.len(), 2, "f{k}: {callees:?}");
                assert_eq!(callees[0], format!("f{}", (k + 1) % n));
                assert!(callees[1].starts_with('v'));
                // the ring is always intact: impact over the cycle reaches every function
                let c: i64 = conn
                    .query_row(
                        "SELECT count(DISTINCT scope) FROM graph_impact(?1, 64, ?2)",
                        sqlite_csr_graph::params![format!("f{k}"), R],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(c, n as i64);
                reads.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    let mut v = 1;
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_secs_f64() < 3.0 {
        for i in 0..n {
            store.upsert_file(&file(i, n, v)).unwrap();
        }
        v += 1;
    }
    stop.store(true, Ordering::Relaxed);
    for h in hs {
        h.join().unwrap();
    }
    assert!(reads.load(Ordering::Relaxed) > 100);
    // wait for any running merge, then fold the rest
    std::thread::sleep(std::time::Duration::from_millis(300));
    let before = common::dump(&store.graph.snapshot(1).unwrap());
    store.merge_now(1).unwrap();
    let after = common::dump(&store.graph.snapshot(1).unwrap());
    assert_eq!(before, after, "merge changed the graph");
    assert!(store.graph.snapshot(1).unwrap().delta.ops.is_empty());
    // merged segments == rebuild from SQLite (source of truth)
    let conn = sqlite_csr_graph::Connection::open(&path).unwrap();
    assert_eq!(after, common::base_edges(&conn, 1));
    eprintln!(
        "versions written={v} reads={}",
        reads.load(Ordering::Relaxed)
    );
}

#[test]
fn delta_replay_after_reopen_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    {
        let s = Store::open(&path).unwrap();
        for i in 0..10 {
            s.upsert_file(&file(i, 10, 0)).unwrap();
        }
        s.merge_now(1).unwrap();
        s.upsert_file(&file(3, 10, 7)).unwrap(); // stays in delta (not merged)
        s.delete_file(R, "src/f5.rs").unwrap();
    }
    let s = Store::open(&path).unwrap(); // reload: segments + replayed delta
    let snap = s.graph.snapshot(1).unwrap();
    assert_eq!(snap.delta.ops.len(), 2);
    let conn = sqlite_csr_graph::Connection::open(&path).unwrap();
    assert_eq!(common::dump(&snap), common::base_edges(&conn, 1));
    assert!(s.graph.callees(1, "f5").is_empty());
    let d = s.graph.dict.load();
    let c: Vec<_> = s
        .graph
        .callees(1, "f3")
        .into_iter()
        .map(|(_, n)| d.name(n).to_string())
        .collect();
    assert_eq!(c, vec!["f4", "v7"]);
}
