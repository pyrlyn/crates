//! Streaming table-valued functions: ORDER BY on the natural row order needs no sorter,
//! LIMIT returns the prefix of the full result, columns that are never read cost nothing,
//! `graph_impact_many`, and cursors that are re-filtered or interleaved.
#![cfg(not(feature = "loadable"))]
use sqlitex_graph::{Connection, DefRow, FileSymbols, RefRow, Store, Value};

const R: &str = "/s";
const N: usize = 20;

fn call(name: &str, scope: &str) -> RefRow {
    RefRow {
        name: name.into(),
        scope: scope.into(),
        kind: "call".into(),
        line: 2,
        ..Default::default()
    }
}

/// `m{i}` calls `m{i+1}` and `leaf`: a chain N levels deep and a name with N direct users.
fn fixture(dir: &std::path::Path) -> Store {
    let store = Store::open(&dir.join("s.db")).unwrap();
    for i in 0..N {
        store
            .upsert_file(&FileSymbols {
                root: R.into(),
                path: format!("src/m{i:02}.rs"),
                defs: vec![DefRow {
                    name: format!("m{i}"),
                    kind: "function".into(),
                    line: 1,
                    end_line: 9,
                    ..Default::default()
                }],
                refs: vec![
                    call(&format!("m{}", i + 1), &format!("m{i}")),
                    call("leaf", &format!("m{i}")),
                ],
                ..Default::default()
            })
            .unwrap();
    }
    store
}

fn rows(c: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut st = c.prepare(sql).unwrap();
    let n = st.column_count();
    st.query_map([], |r| (0..n).map(|i| r.get::<_, Value>(i)).collect())
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn plan(c: &Connection, sql: &str) -> String {
    rows(c, &format!("EXPLAIN QUERY PLAN {sql}"))
        .into_iter()
        .map(|r| format!("{:?}", r[3]))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn order_by_the_natural_order_needs_no_sorter() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(dir.path());
    let c = store.reader().unwrap();
    let q = format!("SELECT depth, path, scope FROM graph_impact('m{N}', 64, '{R}')");
    for consumed in [
        "ORDER BY depth",
        "ORDER BY depth, path",
        "ORDER BY depth, path, scope",
    ] {
        let p = plan(&c, &format!("{q} {consumed}"));
        assert!(!p.contains("TEMP B-TREE"), "{consumed}: {p}");
        assert_eq!(rows(&c, &format!("{q} {consumed}")), rows(&c, &q));
    }
    for sorted in [
        "ORDER BY path",
        "ORDER BY depth DESC",
        "ORDER BY scope, depth",
        "ORDER BY +depth",
        "ORDER BY depth, path COLLATE NOCASE",
    ] {
        let p = plan(&c, &format!("{q} {sorted}"));
        assert!(p.contains("TEMP B-TREE"), "{sorted}: {p}");
    }
    let p = plan(
        &c,
        &format!("SELECT path, scope FROM graph_callers('leaf', '{R}') ORDER BY path, scope"),
    );
    assert!(!p.contains("TEMP B-TREE"), "{p}");
    let p = plan(
        &c,
        &format!(
            "SELECT depth, name FROM graph_neighbors('m0', 3, 'out', '{R}') ORDER BY depth, name"
        ),
    );
    assert!(!p.contains("TEMP B-TREE"), "{p}");
}

#[test]
fn limit_returns_the_prefix_and_count_reads_no_columns() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(dir.path());
    let c = store.reader().unwrap();
    let q = format!("SELECT depth, path, scope FROM graph_impact('m{N}', 64, '{R}')");
    let all = rows(&c, &q);
    assert_eq!(all.len(), N, "one row per level of the chain");
    assert_eq!(all[0][0], Value::Integer(1));
    assert_eq!(rows(&c, &format!("{q} LIMIT 3")), all[..3].to_vec());
    assert_eq!(
        rows(&c, &format!("{q} ORDER BY depth LIMIT 2 OFFSET 4")),
        all[4..6].to_vec()
    );
    let n: i64 = c
        .query_row(
            &format!("SELECT count(*) FROM graph_impact('m{N}', 64, '{R}')"),
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n as usize, N);
    let exists: i64 = c
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM graph_impact('leaf', 64, '{R}'))"),
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exists, 1);
}

#[test]
fn impact_many_takes_one_name_per_line() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(dir.path());
    let c = store.reader().unwrap();
    let got = rows(
        &c,
        &format!(
            "SELECT depth, path, scope FROM graph_impact_many('m5' || char(10) || 'm12' || char(13, 10) || char(10), 2, '{R}')"
        ),
    );
    let want = store.graph.impact_many(1, &["m5", "m12"], 2);
    let d = store.graph.dict.load();
    let want: Vec<Vec<Value>> = want
        .iter()
        .map(|r| {
            vec![
                Value::Integer(r.depth as i64),
                Value::Text(d.path(r.file).into()),
                Value::Text(d.name(r.scope).into()),
            ]
        })
        .collect();
    assert_eq!(got, want);
    assert_eq!(got.len(), 4, "m4, m3 and m11, m10");
    // Every m-name at once: one level-1 row per caller (m0 has none).
    let names: String = c
        .query_row(
            "SELECT group_concat(name, char(10)) FROM names WHERE name LIKE 'm%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let n: i64 = c
        .query_row(
            &format!("SELECT count(*) FROM graph_impact_many(?1, 1, '{R}')"),
            [&names],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n as usize, N);
}

#[test]
fn refiltered_and_interleaved_cursors() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(dir.path());
    let c = store.reader().unwrap();
    // graph_callers as the inner loop: xFilter runs once per outer row on one cursor.
    let joined: i64 = c
        .query_row(
            &format!(
                "SELECT count(*) FROM names n, graph_callers(n.name, '{R}') c WHERE n.name LIKE 'm%'"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(joined as usize, N, "m1..=m{N} have one caller each");
    // Two impact cursors open at once on one thread.
    let pairs: i64 = c
        .query_row(
            &format!(
                "SELECT count(*) FROM graph_impact('leaf', 1, '{R}') a, graph_impact('m{N}', 64, '{R}') b"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pairs as usize, N * N);
}
