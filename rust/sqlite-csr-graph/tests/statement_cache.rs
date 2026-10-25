//! The bounded prepared-statement cache: default capacity, least-recently-used eviction,
//! capacity 0 turning it off, flush, and statements in use staying out of the count.
#![cfg(not(feature = "loadable"))]
use sqlite_csr_graph::Connection;
use sqlite_csr_graph::sqlite::DEFAULT_STATEMENT_CACHE_CAPACITY;

fn one(c: &Connection, sql: &str) -> i64 {
    c.prepare_cached(sql)
        .unwrap()
        .query_row([], |r| r.get(0))
        .unwrap()
}

#[test]
fn the_cache_is_bounded_and_evicts_the_least_recently_used() {
    let c = Connection::open_in_memory().unwrap();
    for i in 0..(DEFAULT_STATEMENT_CACHE_CAPACITY + 10) {
        assert_eq!(one(&c, &format!("SELECT {i}")), i as i64);
    }
    assert_eq!(
        c.prepared_statement_cache_len(),
        DEFAULT_STATEMENT_CACHE_CAPACITY
    );
    c.set_prepared_statement_cache_capacity(2);
    assert_eq!(c.prepared_statement_cache_len(), 2);
    let (a, b, d) = ("SELECT -1", "SELECT -2", "SELECT -3");
    one(&c, a);
    one(&c, b);
    one(&c, a); // a is now the most recent
    one(&c, d); // evicts b
    assert_eq!(c.prepared_statement_cache_len(), 2);
    // A hit takes the statement out of the cache while it is in use.
    let held = c.prepare_cached(a).unwrap();
    assert_eq!(c.prepared_statement_cache_len(), 1);
    let fresh = c.prepare_cached(b).unwrap();
    assert_eq!(c.prepared_statement_cache_len(), 1, "b was evicted: a miss");
    drop((held, fresh));
    assert_eq!(c.prepared_statement_cache_len(), 2);
}

#[test]
fn capacity_zero_turns_the_cache_off_and_flush_empties_it() {
    let c = Connection::open_in_memory().unwrap();
    for i in 0..5 {
        one(&c, &format!("SELECT {i}"));
    }
    assert_eq!(c.prepared_statement_cache_len(), 5);
    c.flush_prepared_statement_cache();
    assert_eq!(c.prepared_statement_cache_len(), 0);
    c.set_prepared_statement_cache_capacity(0);
    for _ in 0..3 {
        assert_eq!(one(&c, "SELECT 7"), 7);
        assert_eq!(c.prepared_statement_cache_len(), 0);
    }
}

#[test]
fn cached_statements_survive_schema_changes() {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1), (2);")
        .unwrap();
    assert_eq!(one(&c, "SELECT count(*) FROM t"), 2);
    c.execute_batch("CREATE TABLE u(y); INSERT INTO t VALUES (3);")
        .unwrap();
    assert_eq!(one(&c, "SELECT count(*) FROM t"), 3);
}
