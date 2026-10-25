//! `disable_sqlite_memstatus` works only before SQLite is initialised. Its own test binary
//! with a single test, so nothing else in the process has opened a connection first.
#![cfg(not(feature = "loadable"))]

#[test]
fn memstatus_can_be_turned_off_only_before_the_first_connection() {
    assert!(sqlite_csr_graph::disable_sqlite_memstatus());
    let c = sqlite_csr_graph::Connection::open_in_memory().unwrap();
    let n: i64 = c.query_row("SELECT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
    assert!(
        !sqlite_csr_graph::disable_sqlite_memstatus(),
        "SQLite is initialised now; the call must report it had no effect"
    );
}
