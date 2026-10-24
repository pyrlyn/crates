//! The SQL surface on a small fixture graph built through `Store`: impact, neighbors,
//! callers, callees, stats, merge and the lazily loaded graph of `register`.
use sqlite_csr_graph::{DefRow, FileSymbols, RefRow, Store};
use std::collections::BTreeSet;

const R: &str = "/fixture";

fn def(name: &str) -> DefRow {
    DefRow {
        name: name.into(),
        kind: "function".into(),
        line: 1,
        end_line: 9,
        ..Default::default()
    }
}

fn call(name: &str, scope: &str) -> RefRow {
    RefRow {
        name: name.into(),
        scope: scope.into(),
        kind: "call".into(),
        line: 2,
        ..Default::default()
    }
}

/// main -> parse -> lex, main -> run -> parse; util.rs imports `lex`.
fn fixture(dir: &std::path::Path) -> (std::path::PathBuf, Store) {
    let path = dir.join("g.db");
    let store = Store::open(&path).unwrap();
    let files = [
        FileSymbols {
            root: R.into(),
            path: "main.rs".into(),
            defs: vec![def("main")],
            refs: vec![call("parse", "main"), call("run", "main")],
            ..Default::default()
        },
        FileSymbols {
            root: R.into(),
            path: "run.rs".into(),
            defs: vec![def("run")],
            refs: vec![call("parse", "run")],
            ..Default::default()
        },
        FileSymbols {
            root: R.into(),
            path: "parse.rs".into(),
            defs: vec![def("parse"), def("lex")],
            refs: vec![call("lex", "parse")],
            ..Default::default()
        },
        FileSymbols {
            root: R.into(),
            path: "util.rs".into(),
            defs: vec![def("helper")],
            refs: vec![RefRow {
                name: "lex".into(),
                scope: String::new(),
                kind: "import".into(),
                line: 1,
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    for f in &files {
        store.upsert_file(f).unwrap();
    }
    (path, store)
}

fn strings(
    conn: &sqlite_csr_graph::Connection,
    sql: &str,
    p: impl sqlite_csr_graph::sqlite::Params,
) -> BTreeSet<String> {
    let mut st = conn.prepare(sql).unwrap();
    let n = st.column_count();
    st.query_map(p, |r| {
        let v: Vec<String> = (0..n)
            .map(|i| {
                r.get::<_, sqlite_csr_graph::Value>(i)
                    .map(|v| format!("{v:?}"))
            })
            .collect::<Result<_, _>>()?;
        Ok(v.join("|"))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

#[test]
fn callers_and_callees_follow_the_written_refs() {
    let dir = tempfile::tempdir().unwrap();
    let (_p, store) = fixture(dir.path());
    let c = store.reader().unwrap();
    let callers = strings(
        &c,
        "SELECT path, scope FROM graph_callers('parse', ?1)",
        [R],
    );
    assert_eq!(
        callers,
        BTreeSet::from([
            "Text(\"main.rs\")|Text(\"main\")".to_string(),
            "Text(\"run.rs\")|Text(\"run\")".to_string()
        ])
    );
    let callees = strings(&c, "SELECT callee FROM graph_callees('main', ?1)", [R]);
    assert_eq!(
        callees,
        BTreeSet::from(["Text(\"parse\")".to_string(), "Text(\"run\")".to_string()])
    );
}

#[test]
fn impact_reaches_transitive_users_and_importers_once() {
    let dir = tempfile::tempdir().unwrap();
    let (_p, store) = fixture(dir.path());
    let c = store.reader().unwrap();
    let scopes = strings(&c, "SELECT scope FROM graph_impact('lex', 3, ?1)", [R]);
    for s in ["parse", "main", "run", "helper"] {
        assert!(
            scopes.contains(&format!("Text(\"{s}\")")),
            "{s} missing from {scopes:?}"
        );
    }
    let depth1: i64 = c
        .query_row(
            "SELECT count(*) FROM graph_impact('lex', 1, ?1) WHERE scope='main'",
            [R],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(depth1, 0, "main is two hops from lex");
}

#[test]
fn neighbors_walk_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let (_p, store) = fixture(dir.path());
    let c = store.reader().unwrap();
    let out = strings(
        &c,
        "SELECT name FROM graph_neighbors('main', 2, 'out', ?1)",
        [R],
    );
    assert!(out.contains("Text(\"lex\")"), "{out:?}");
    let inn = strings(
        &c,
        "SELECT name FROM graph_neighbors('parse', 1, 'in', ?1)",
        [R],
    );
    assert!(
        inn.contains("Text(\"main\")") && inn.contains("Text(\"run\")"),
        "{inn:?}"
    );
}

#[test]
fn merge_keeps_results_and_register_loads_the_persisted_graph() {
    let dir = tempfile::tempdir().unwrap();
    let (path, store) = fixture(dir.path());
    let c = store.reader().unwrap();
    let before = strings(
        &c,
        "SELECT depth, path, scope FROM graph_impact('lex', 4, ?1)",
        [R],
    );
    let merged: i64 = c
        .query_row("SELECT graph_merge(?1)", [R], |r| r.get(0))
        .unwrap();
    assert_eq!(merged, 1);
    assert_eq!(
        before,
        strings(
            &c,
            "SELECT depth, path, scope FROM graph_impact('lex', 4, ?1)",
            [R]
        )
    );
    let stats: String = c
        .query_row("SELECT graph_stats()", [], |r| r.get(0))
        .unwrap();
    assert!(
        stats.contains("\"delta_ops\":0"),
        "merge left a delta: {stats}"
    );
    drop((c, store));
    // A plain connection with `register` (the loadable extension's path) sees the same graph.
    let plain = sqlite_csr_graph::Connection::open(&path).unwrap();
    sqlite_csr_graph::register(&plain).unwrap();
    plain
        .query_row("SELECT graph_reload()", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert_eq!(
        before,
        strings(
            &plain,
            "SELECT depth, path, scope FROM graph_impact('lex', 4, ?1)",
            [R]
        )
    );
}

#[test]
fn unknown_names_and_roots_return_no_rows() {
    let dir = tempfile::tempdir().unwrap();
    let (_p, store) = fixture(dir.path());
    let c = store.reader().unwrap();
    assert!(strings(&c, "SELECT * FROM graph_callers('nope', ?1)", [R]).is_empty());
    assert!(strings(&c, "SELECT * FROM graph_impact('lex', 3, '/elsewhere')", []).is_empty());
}

#[test]
fn register_refuses_in_memory_databases() {
    let c = sqlite_csr_graph::Connection::open_in_memory().unwrap();
    assert!(sqlite_csr_graph::register(&c).is_err());
}
