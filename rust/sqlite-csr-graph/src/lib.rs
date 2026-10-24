//! sqlite-csr-graph: a code-symbol store in SQLite with a native call/reference graph kept in
//! compact CSR segments, queried through table-valued functions.
//! See README.md.

mod ffi;
pub mod graph;
pub mod segment;
pub mod store;
pub mod vtab;

/// The safe SQLite wrapper this crate is built on (over `libsqlite3-sys`).
pub mod sqlite {
    pub use crate::ffi::{
        CachedStatement, Connection, Context, Error, FromSql, FunctionFlags, OpenFlags,
        OptionalExtension, Params, Result, Row, RowIndex, Rows, Statement, ToSql, Transaction,
        TransactionBehavior, Value,
    };
}

pub use ffi::{Connection, Error, Value};
pub use graph::Graph;
pub use store::{DefRow, FileSymbols, RefRow, Store};

#[cfg(feature = "rtok-import")]
pub mod rtok;
#[cfg(feature = "rtok-import")]
pub use rtok::import_rtok;
pub use vtab::{register, register_with};

#[cfg(feature = "loadable")]
pub use ffi::sqlite3_extension_init;
#[cfg(not(feature = "loadable"))]
pub use ffi::{cancel_auto_extension, register_auto_extension};

/// Turn off SQLite's global memory statistics (`SQLITE_CONFIG_MEMSTATUS`). With it on (the
/// bundled default) every sqlite3_malloc takes one process-wide mutex, which caps multi-thread
/// read throughput. Must be called before the first connection is opened; returns false if
/// SQLite was already initialised. Not available in the loadable build (the host owns SQLite).
#[cfg(not(feature = "loadable"))]
pub fn disable_sqlite_memstatus() -> bool {
    ffi::disable_memstatus()
}
