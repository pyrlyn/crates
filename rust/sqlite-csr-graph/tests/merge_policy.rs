//! Merge thresholds: `StoreOptions`, the runtime setters, `auto_merge = false`, and a
//! merge that finished second not overwriting the newer base of the one that finished first.
#![cfg(not(feature = "loadable"))]
mod common;
use sqlite_csr_graph::store::DEFAULT_MERGE_THRESHOLD;
use sqlite_csr_graph::{DefRow, FileSymbols, RefRow, Store, StoreOptions};
use std::time::{Duration, Instant};

const R: &str = "/m";

fn file(i: u32, v: u32) -> FileSymbols {
    FileSymbols {
        root: R.into(),
        path: format!("src/f{i}.rs"),
        defs: vec![DefRow {
            name: format!("f{i}"),
            kind: "function".into(),
            line: 1,
            end_line: 5,
            ..Default::default()
        }],
        refs: vec![RefRow {
            name: format!("g{v}"),
            scope: format!("f{i}"),
            kind: "call".into(),
            line: 2,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn delta_ops(s: &Store) -> usize {
    s.graph.snapshot(1).map_or(0, |s| s.delta.ops.len())
}

#[test]
fn defaults_and_setters() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("a.db")).unwrap();
    assert_eq!(
        s.options(),
        StoreOptions {
            merge_threshold: DEFAULT_MERGE_THRESHOLD,
            auto_merge: true
        }
    );
    s.set_merge_threshold(5);
    s.set_auto_merge(false);
    assert_eq!(
        s.options(),
        StoreOptions {
            merge_threshold: 5,
            auto_merge: false
        }
    );
}

#[test]
fn auto_merge_off_keeps_every_write_in_the_delta() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
    let s = Store::open_with(
        &path,
        StoreOptions {
            merge_threshold: 0,
            auto_merge: false,
        },
    )
    .unwrap();
    for i in 0..40 {
        s.upsert_file(&file(i, 0)).unwrap();
    }
    assert_eq!(delta_ops(&s), 40);
    s.merge_now(1).unwrap();
    assert_eq!(delta_ops(&s), 0);
    let conn = sqlite_csr_graph::Connection::open(&path).unwrap();
    assert_eq!(
        common::dump(&s.graph.snapshot(1).unwrap()),
        common::base_edges(&conn, 1)
    );
}

#[test]
fn a_low_threshold_merges_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.db");
    let s = Store::open_with(
        &path,
        StoreOptions {
            merge_threshold: 1,
            auto_merge: true,
        },
    )
    .unwrap();
    for i in 0..10 {
        s.upsert_file(&file(i, 1)).unwrap();
    }
    // Merges run on their own thread (one at a time, so the last writes may still be in
    // the delta); wait until one has installed a base.
    let t0 = Instant::now();
    while s.graph.snapshot(1).unwrap().base.users.len_items() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(10), "no merge ran");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Raising the threshold stops further merges.
    s.set_merge_threshold(usize::MAX);
    std::thread::sleep(Duration::from_millis(100));
    let before = delta_ops(&s);
    for i in 0..10 {
        s.upsert_file(&file(i, 2)).unwrap();
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(delta_ops(&s), before + 10);
    s.merge_now(1).unwrap();
    let conn = sqlite_csr_graph::Connection::open(&path).unwrap();
    assert_eq!(
        common::dump(&s.graph.snapshot(1).unwrap()),
        common::base_edges(&conn, 1)
    );
}

#[test]
fn the_merge_that_finishes_second_does_not_replace_a_newer_base() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.db");
    let s = Store::open_with(
        &path,
        StoreOptions {
            merge_threshold: usize::MAX,
            auto_merge: false,
        },
    )
    .unwrap();
    for i in 0..5 {
        s.upsert_file(&file(i, 0)).unwrap();
    }
    let old = s.graph.snapshot(1).unwrap();
    for i in 5..8 {
        s.upsert_file(&file(i, 0)).unwrap();
    }
    let new = s.graph.snapshot(1).unwrap();
    assert!(s.graph.install_merged(1, &new, new.compact()));
    // Built from the older snapshot: it lacks files 5..8, which are no longer in the delta.
    assert!(!s.graph.install_merged(1, &old, old.compact()));
    let conn = sqlite_csr_graph::Connection::open(&path).unwrap();
    assert_eq!(
        common::dump(&s.graph.snapshot(1).unwrap()),
        common::base_edges(&conn, 1)
    );
}
