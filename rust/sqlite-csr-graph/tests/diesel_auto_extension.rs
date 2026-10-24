//! A diesel host (how rtok consumes the crate): `register_auto_extension` once, then every
//! `SqliteConnection` diesel opens on the store's file has the graph functions.
#![cfg(not(feature = "loadable"))]
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use sqlite_csr_graph::{DefRow, FileSymbols, RefRow, Store};

#[derive(QueryableByName, Debug, PartialEq)]
struct Caller {
    #[diesel(sql_type = Text)]
    path: String,
    #[diesel(sql_type = Text)]
    scope: String,
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

#[test]
fn diesel_connections_get_the_functions_through_auto_extension() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .upsert_file(&FileSymbols {
                root: "/r".into(),
                path: "main.rs".into(),
                defs: vec![DefRow {
                    name: "main".into(),
                    kind: "function".into(),
                    line: 1,
                    end_line: 3,
                    ..Default::default()
                }],
                refs: vec![RefRow {
                    name: "run".into(),
                    scope: "main".into(),
                    kind: "call".into(),
                    line: 2,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
    }
    sqlite_csr_graph::register_auto_extension().unwrap();
    let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    diesel::sql_query("SELECT graph_reload()")
        .execute(&mut conn)
        .unwrap();
    let rows: Vec<Caller> = diesel::sql_query("SELECT path, scope FROM graph_callers('run', '/r')")
        .load(&mut conn)
        .unwrap();
    assert_eq!(
        rows,
        vec![Caller {
            path: "main.rs".into(),
            scope: "main".into()
        }]
    );
    let n: Count = diesel::sql_query("SELECT count(*) AS n FROM graph_impact('run', 3, '/r')")
        .get_result(&mut conn)
        .unwrap();
    assert_eq!(n.n, 1);
    // In-memory connections are skipped, not failed.
    SqliteConnection::establish(":memory:").unwrap();
    assert!(sqlite_csr_graph::cancel_auto_extension());
}
