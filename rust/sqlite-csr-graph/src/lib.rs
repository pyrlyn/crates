//! sqlite-csr-graph: a code-symbol store in SQLite with a native call/reference graph kept in
//! compact CSR segments, queried through table-valued functions.
//! See README.md.

pub mod graph;
pub mod segment;
pub mod store;
pub mod vtab;

pub use graph::Graph;
pub use store::{DefRow, FileSymbols, RefRow, Store};

#[cfg(feature = "rtok-import")]
pub mod rtok;
#[cfg(feature = "rtok-import")]
pub use rtok::import_rtok;
pub use vtab::{register, register_with};

/// SQLite loadable-extension entry point (`.load libsqlite_csr_graph` / `load_extension`).
/// Built with `--no-default-features --features loadable`.
///
/// # Safety
///
/// Called by SQLite only (`sqlite3_load_extension`), with the `db`, error-message and API
/// pointers SQLite passes to an extension entry point.
#[cfg(feature = "loadable")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite3_extension_init(
    db: *mut rusqlite::ffi::sqlite3,
    pz_err_msg: *mut *mut std::ffi::c_char,
    p_api: *mut rusqlite::ffi::sqlite3_api_routines,
) -> std::ffi::c_int {
    fn init(conn: rusqlite::Connection) -> rusqlite::Result<bool> {
        vtab::register(&conn)?;
        Ok(false)
    }
    unsafe { rusqlite::Connection::extension_init2(db, pz_err_msg, p_api, init) }
}

/// Turn off SQLite's global memory statistics (`SQLITE_CONFIG_MEMSTATUS`). With it on (the
/// bundled default) every sqlite3_malloc takes one process-wide mutex, which caps multi-thread
/// read throughput. Must be called before the first connection is opened; returns false if
/// SQLite was already initialised. Not available in the loadable build (the host owns SQLite).
#[cfg(not(feature = "loadable"))]
#[allow(unsafe_code)]
pub fn disable_sqlite_memstatus() -> bool {
    // SAFETY: sqlite3_config is only valid before sqlite3_initialize; SQLite returns
    // SQLITE_MISUSE (no effect) otherwise, which we report as false.
    unsafe {
        rusqlite::ffi::sqlite3_config(rusqlite::ffi::SQLITE_CONFIG_MEMSTATUS, 0)
            == rusqlite::ffi::SQLITE_OK
    }
}
