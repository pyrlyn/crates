//! A change feed for a SQLite database shared between processes: did another
//! connection commit since I last looked?
//!
//! The feed polls SQLite's `PRAGMA data_version`, which changes when another
//! connection — in practice another process — commits, and stays put for
//! this connection's own commits. That is what a caller sharing a database
//! file wants: it already knows about its own writes. The version does not
//! say which table changed, so a `true` means "re-read what you show".
//! Pull-based: no thread, no timer; the caller polls as often as it
//! refreshes. Extracted from cox's `cox-store` (`watch.rs`).
//!
//! The version is per connection, so a [`ChangeToken`] is meaningful only
//! with the connection that issued it. Each consumer holds its own token, so
//! two pollers never consume each other's notification.
//!
//! ```
//! use diesel::{Connection, RunQueryDsl, SqliteConnection};
//! use sqlite_change_feed::ChangeToken;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let dir = tempfile::tempdir()?;
//! let url = dir.path().join("app.db").display().to_string();
//! let mut mine = SqliteConnection::establish(&url)?;
//! let mut theirs = SqliteConnection::establish(&url)?;
//!
//! let mut token = ChangeToken::new(&mut mine)?;
//! diesel::sql_query("CREATE TABLE t (x INTEGER)").execute(&mut theirs)?;
//! assert!(token.poll(&mut mine)?);
//! assert!(!token.poll(&mut mine)?);
//! # Ok(())
//! # }
//! ```

use diesel::sql_types::BigInt;
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};

/// Why the feed could not read the database's version.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// `PRAGMA data_version` failed on the connection.
    #[error("reading PRAGMA data_version: {0}")]
    Query(#[from] diesel::result::Error),
}

/// A position in the feed: the `data_version` a consumer last saw on one
/// connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeToken(i64);

impl ChangeToken {
    /// The feed's current position on `conn`; the next [`ChangeToken::poll`]
    /// reports only commits made after this call.
    ///
    /// # Errors
    ///
    /// [`Error::Query`] when the pragma fails.
    pub fn new(conn: &mut SqliteConnection) -> Result<Self, Error> {
        data_version(conn).map(Self)
    }

    /// Whether another connection committed since this token was taken or
    /// last polled, and advances it. Commits made through `conn` itself never
    /// report `true`.
    ///
    /// # Errors
    ///
    /// [`Error::Query`] when the pragma fails; the token is left as it was.
    pub fn poll(&mut self, conn: &mut SqliteConnection) -> Result<bool, Error> {
        let now = data_version(conn)?;
        let changed = now != self.0;
        self.0 = now;
        Ok(changed)
    }
}

#[derive(QueryableByName)]
struct DataVersion {
    #[diesel(sql_type = BigInt)]
    data_version: i64,
}

/// `PRAGMA data_version` on `conn`: a number that changes whenever another
/// connection commits to the same database.
///
/// # Errors
///
/// [`Error::Query`] when the pragma fails.
pub fn data_version(conn: &mut SqliteConnection) -> Result<i64, Error> {
    // Raw SQL because Diesel cannot model a PRAGMA.
    let row = diesel::sql_query("PRAGMA data_version").get_result::<DataVersion>(conn)?;
    Ok(row.data_version)
}
