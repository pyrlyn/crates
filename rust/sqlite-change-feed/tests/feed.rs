//! The feed against real database files: a second connection in this
//! process, and a writer in another process.
//!
//! The cross-process test re-executes this test binary: the parent spawns
//! `std::env::current_exe()` running only the ignored `writer_process` test,
//! with `WRITER_DB` naming the database to write. That is lighter than a
//! test-only `[[bin]]` or example, which would be one more target to build
//! and locate; this binary is already built and on disk.

use std::path::{Path, PathBuf};
use std::process::Command;

use diesel::{Connection, RunQueryDsl, SqliteConnection};
use sqlite_change_feed::{ChangeToken, data_version};

const WRITER_DB: &str = "SQLITE_CHANGE_FEED_TEST_WRITER_DB";

fn open(path: &Path) -> SqliteConnection {
    let mut conn = SqliteConnection::establish(&path.display().to_string()).expect("open");
    // Two writers on one file wait for each other instead of failing busy.
    diesel::sql_query("PRAGMA busy_timeout = 5000")
        .execute(&mut conn)
        .expect("busy_timeout");
    conn
}

fn create(conn: &mut SqliteConnection) {
    diesel::sql_query("CREATE TABLE IF NOT EXISTS notes (body TEXT NOT NULL)")
        .execute(conn)
        .expect("create");
}

fn insert(conn: &mut SqliteConnection, body: &str) {
    diesel::sql_query("INSERT INTO notes (body) VALUES (?)")
        .bind::<diesel::sql_types::Text, _>(body)
        .execute(conn)
        .expect("insert");
}

/// The child side of `another_process_commit_moves_the_feed_once`: opens the
/// database named by `WRITER_DB` and commits one row.
#[test]
#[ignore = "run by another_process_commit_moves_the_feed_once in a child process"]
fn writer_process() {
    let path = PathBuf::from(std::env::var_os(WRITER_DB).expect("writer db"));
    let mut conn = open(&path);
    insert(&mut conn, "from the child");
}

#[test]
fn another_process_commit_moves_the_feed_once() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("shared.db");
    let mut conn = open(&path);
    create(&mut conn);
    let mut token = ChangeToken::new(&mut conn).expect("token");
    assert!(!token.poll(&mut conn).expect("poll"), "nothing yet");

    let out = Command::new(std::env::current_exe().expect("current_exe"))
        .args([
            "writer_process",
            "--exact",
            "--ignored",
            "--test-threads=1",
            "-q",
        ])
        .env(WRITER_DB, &path)
        .output()
        .expect("spawn writer");
    assert!(out.status.success(), "the writer failed: {out:?}");

    assert!(
        token.poll(&mut conn).expect("poll"),
        "sees the other process's commit"
    );
    assert!(!token.poll(&mut conn).expect("poll"), "reported once");
    let count: i64 = diesel::select(diesel::dsl::sql::<diesel::sql_types::BigInt>(
        "(SELECT COUNT(*) FROM notes)",
    ))
    .get_result(&mut conn)
    .expect("count");
    assert_eq!(count, 1, "the child's row landed");
}

#[test]
fn a_second_connection_commit_is_seen_and_own_commits_are_not() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("shared.db");
    let (mut mine, mut theirs) = (open(&path), open(&path));
    create(&mut mine);
    let mut token = ChangeToken::new(&mut mine).expect("token");

    insert(&mut mine, "own write");
    assert!(
        !token.poll(&mut mine).expect("poll"),
        "own commits do not count"
    );

    insert(&mut theirs, "their write");
    assert!(
        token.poll(&mut mine).expect("poll"),
        "another connection's commit"
    );
    assert!(!token.poll(&mut mine).expect("poll"), "reported once");
}

#[test]
fn each_consumer_holds_its_own_position() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("shared.db");
    let (mut mine, mut theirs) = (open(&path), open(&path));
    create(&mut mine);
    let mut sidebar = ChangeToken::new(&mut mine).expect("token");
    let mut totals = ChangeToken::new(&mut mine).expect("token");

    insert(&mut theirs, "their write");
    assert!(sidebar.poll(&mut mine).expect("poll"));
    assert!(
        totals.poll(&mut mine).expect("poll"),
        "not consumed by the other poller"
    );
}

#[test]
fn a_private_in_memory_database_never_changes_under_its_connection() {
    let mut conn = SqliteConnection::establish(":memory:").expect("memory");
    let before = data_version(&mut conn).expect("version");
    create(&mut conn);
    insert(&mut conn, "own write");
    assert_eq!(data_version(&mut conn).expect("version"), before);
}
