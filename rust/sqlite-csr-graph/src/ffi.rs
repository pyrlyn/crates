//! The only module with `unsafe`: a small safe wrapper over `libsqlite3-sys` (connection,
//! prepared statements, values, scalar functions), eponymous table-valued function modules
//! with hand-written `xBestIndex`/`xFilter`/`xNext`/`xEof`/`xColumn`/`xRowid`, the loadable
//! extension entry point and `sqlite3_auto_extension` registration.
//!
//! Every callback SQLite calls runs inside `catch_unwind`: a panic becomes `SQLITE_ERROR`
//! (or a function error) and never unwinds into C.
#![allow(unsafe_code)]

use libsqlite3_sys as ffi;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::ops::{BitOr, Deref, DerefMut};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

// ---------------------------------------------------------------- errors

/// Errors from SQLite or from this crate's wrapper.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// SQLite returned `code` (extended result code) with `message`.
    Sqlite {
        /// Extended result code.
        code: i32,
        /// `sqlite3_errmsg` text.
        message: String,
    },
    /// `query_row` found no row.
    QueryReturnedNoRows,
    /// A column or argument held a type the caller cannot convert from.
    InvalidType {
        /// Column or argument index.
        index: usize,
        /// The requested Rust type.
        expected: &'static str,
    },
    /// A string handed to SQLite contained a NUL byte.
    NulInString,
    /// Error raised by a table-valued function module.
    Module(String),
    /// Error raised by a user (scalar or table) function.
    User(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Sqlite { code, message } => write!(f, "sqlite error {code}: {message}"),
            Error::QueryReturnedNoRows => f.write_str("query returned no rows"),
            Error::InvalidType { index, expected } => {
                write!(f, "column {index}: value is not a {expected}")
            }
            Error::NulInString => f.write_str("string contains a NUL byte"),
            Error::Module(m) | Error::User(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

/// `Result` with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Turn `Err(QueryReturnedNoRows)` into `Ok(None)`.
pub trait OptionalExtension<T> {
    /// See the trait docs.
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn cstring(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::NulInString)
}

fn len_c(n: usize) -> Result<c_int> {
    c_int::try_from(n).map_err(|_| Error::Sqlite {
        code: ffi::SQLITE_TOOBIG,
        message: "value too large".into(),
    })
}

/// Copy `msg` into memory from `sqlite3_malloc`, for SQLite to free.
fn sqlite_string(msg: &str) -> *mut c_char {
    let bytes = msg.as_bytes();
    let n = bytes.len().min(1 << 20);
    // SAFETY: sqlite3_malloc returns at least n+1 writable bytes or null; we write exactly n+1.
    unsafe {
        let p = ffi::sqlite3_malloc((n + 1) as c_int) as *mut u8;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, n);
            *p.add(n) = 0;
        }
        p as *mut c_char
    }
}

fn db_error(db: *mut ffi::sqlite3, code: c_int) -> Error {
    // SAFETY: `db` is a live handle (or null, which sqlite3_errmsg accepts); the message is
    // copied before any other call on this handle.
    let message = unsafe {
        let m = ffi::sqlite3_errmsg(db);
        if m.is_null() {
            String::new()
        } else {
            CStr::from_ptr(m).to_string_lossy().into_owned()
        }
    };
    let code = if db.is_null() {
        code
    } else {
        // SAFETY: live handle.
        unsafe { ffi::sqlite3_extended_errcode(db) }
    };
    Error::Sqlite { code, message }
}

// ---------------------------------------------------------------- values

/// An owned SQLite value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `NULL`
    Null,
    /// `INTEGER`
    Integer(i64),
    /// `REAL`
    Real(f64),
    /// `TEXT`
    Text(String),
    /// `BLOB`
    Blob(Vec<u8>),
}

/// Something that can be bound as a parameter or returned from a function.
pub trait ToSql {
    /// The value to bind.
    fn to_sql(&self) -> Value;
}

impl ToSql for Value {
    fn to_sql(&self) -> Value {
        self.clone()
    }
}
impl<T: ToSql + ?Sized> ToSql for &T {
    fn to_sql(&self) -> Value {
        (**self).to_sql()
    }
}
impl<T: ToSql> ToSql for Option<T> {
    fn to_sql(&self) -> Value {
        self.as_ref().map_or(Value::Null, T::to_sql)
    }
}
macro_rules! int_to_sql {
    ($($t:ty),*) => {$(impl ToSql for $t { fn to_sql(&self) -> Value { Value::Integer(*self as i64) } })*};
}
int_to_sql!(i8, i16, i32, i64, isize, u8, u16, u32, usize, u64);
impl ToSql for bool {
    fn to_sql(&self) -> Value {
        Value::Integer(*self as i64)
    }
}
impl ToSql for f64 {
    fn to_sql(&self) -> Value {
        Value::Real(*self)
    }
}
impl ToSql for str {
    fn to_sql(&self) -> Value {
        Value::Text(self.to_owned())
    }
}
impl ToSql for std::borrow::Cow<'_, str> {
    fn to_sql(&self) -> Value {
        Value::Text(self.to_string())
    }
}
impl ToSql for String {
    fn to_sql(&self) -> Value {
        Value::Text(self.clone())
    }
}
impl ToSql for [u8] {
    fn to_sql(&self) -> Value {
        Value::Blob(self.to_vec())
    }
}
impl ToSql for Vec<u8> {
    fn to_sql(&self) -> Value {
        Value::Blob(self.clone())
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Value {
        Value::Integer(v)
    }
}
impl From<String> for Value {
    fn from(v: String) -> Value {
        Value::Text(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Value {
        Value::Text(v.into())
    }
}

/// Something a column or function argument converts into.
pub trait FromSql: Sized {
    /// Convert; `index` is only for the error.
    fn from_sql(v: Value, index: usize) -> Result<Self>;
}

fn bad<T>(index: usize) -> Result<T> {
    Err(Error::InvalidType {
        index,
        expected: std::any::type_name::<T>(),
    })
}

impl FromSql for Value {
    fn from_sql(v: Value, _: usize) -> Result<Self> {
        Ok(v)
    }
}
impl<T: FromSql> FromSql for Option<T> {
    fn from_sql(v: Value, i: usize) -> Result<Self> {
        match v {
            Value::Null => Ok(None),
            v => T::from_sql(v, i).map(Some),
        }
    }
}
macro_rules! int_from_sql {
    ($($t:ty),*) => {$(impl FromSql for $t {
        fn from_sql(v: Value, i: usize) -> Result<Self> {
            match v { Value::Integer(n) => <$t>::try_from(n).or_else(|_| bad(i)), _ => bad(i) }
        }
    })*};
}
int_from_sql!(i8, i16, i32, i64, isize, u8, u16, u32, usize, u64);
impl FromSql for bool {
    fn from_sql(v: Value, i: usize) -> Result<Self> {
        i64::from_sql(v, i).map(|n| n != 0)
    }
}
impl FromSql for f64 {
    fn from_sql(v: Value, i: usize) -> Result<Self> {
        match v {
            Value::Real(x) => Ok(x),
            Value::Integer(n) => Ok(n as f64),
            _ => bad(i),
        }
    }
}
impl FromSql for String {
    fn from_sql(v: Value, i: usize) -> Result<Self> {
        match v {
            Value::Text(s) => Ok(s),
            _ => bad(i),
        }
    }
}
impl FromSql for Vec<u8> {
    fn from_sql(v: Value, i: usize) -> Result<Self> {
        match v {
            Value::Blob(b) => Ok(b),
            Value::Text(s) => Ok(s.into_bytes()),
            _ => bad(i),
        }
    }
}

/// Statement parameters: `[]`, `[a, b]` (one type), `params![a, b]` or `&[&dyn ToSql]`.
pub trait Params {
    /// The values, in `?1, ?2, ...` order.
    fn values(self) -> Vec<Value>;
}
impl Params for () {
    fn values(self) -> Vec<Value> {
        vec![]
    }
}
impl Params for &[&dyn ToSql] {
    fn values(self) -> Vec<Value> {
        self.iter().map(|p| p.to_sql()).collect()
    }
}
impl<const N: usize> Params for &[&dyn ToSql; N] {
    fn values(self) -> Vec<Value> {
        self.iter().map(|p| p.to_sql()).collect()
    }
}
impl Params for [&dyn ToSql; 0] {
    fn values(self) -> Vec<Value> {
        vec![]
    }
}
macro_rules! array_params {
    ($($n:literal)*) => {$(impl<T: ToSql> Params for [T; $n] {
        fn values(self) -> Vec<Value> { self.iter().map(ToSql::to_sql).collect() }
    })*};
}
array_params!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);

/// Mixed-type parameters: `params![a, b, c]`.
#[macro_export]
macro_rules! params {
    () => { &[] as &[&dyn $crate::sqlite::ToSql] };
    ($($p:expr),+ $(,)?) => { &[$(&$p as &dyn $crate::sqlite::ToSql),+] as &[&dyn $crate::sqlite::ToSql] };
}

/// # Safety
/// `v` must be a valid `sqlite3_value` for the duration of the call.
unsafe fn value_of(v: *mut ffi::sqlite3_value) -> Value {
    // SAFETY: caller guarantees `v`; text/blob pointers are read before any other call on v.
    unsafe {
        match ffi::sqlite3_value_type(v) {
            ffi::SQLITE_INTEGER => Value::Integer(ffi::sqlite3_value_int64(v)),
            ffi::SQLITE_FLOAT => Value::Real(ffi::sqlite3_value_double(v)),
            ffi::SQLITE_TEXT => {
                let p = ffi::sqlite3_value_text(v);
                let n = ffi::sqlite3_value_bytes(v) as usize;
                Value::Text(if p.is_null() {
                    String::new()
                } else {
                    String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned()
                })
            }
            ffi::SQLITE_BLOB => {
                let p = ffi::sqlite3_value_blob(v) as *const u8;
                let n = ffi::sqlite3_value_bytes(v) as usize;
                Value::Blob(if p.is_null() {
                    vec![]
                } else {
                    std::slice::from_raw_parts(p, n).to_vec()
                })
            }
            _ => Value::Null,
        }
    }
}

/// # Safety
/// `ctx` must be the live context of the current function call.
unsafe fn set_result(ctx: *mut ffi::sqlite3_context, v: &Value) {
    // SAFETY: caller guarantees ctx; SQLITE_TRANSIENT makes SQLite copy text and blobs.
    unsafe {
        match v {
            Value::Null => ffi::sqlite3_result_null(ctx),
            Value::Integer(n) => ffi::sqlite3_result_int64(ctx, *n),
            Value::Real(x) => ffi::sqlite3_result_double(ctx, *x),
            Value::Text(s) => match len_c(s.len()) {
                Ok(n) => ffi::sqlite3_result_text(
                    ctx,
                    s.as_ptr() as *const c_char,
                    n,
                    ffi::SQLITE_TRANSIENT(),
                ),
                Err(_) => ffi::sqlite3_result_error_toobig(ctx),
            },
            Value::Blob(b) => match len_c(b.len()) {
                Ok(n) => ffi::sqlite3_result_blob(
                    ctx,
                    b.as_ptr() as *const c_void,
                    n,
                    ffi::SQLITE_TRANSIENT(),
                ),
                Err(_) => ffi::sqlite3_result_error_toobig(ctx),
            },
        }
    }
}

/// # Safety
/// `ctx` must be the live context of the current function call.
unsafe fn set_error(ctx: *mut ffi::sqlite3_context, msg: &str) {
    let n = msg.len().min(c_int::MAX as usize) as c_int;
    // SAFETY: SQLite copies the message (length given explicitly).
    unsafe { ffi::sqlite3_result_error(ctx, msg.as_ptr() as *const c_char, n) }
}

// ---------------------------------------------------------------- connection

/// Flags for [`Connection::open_with_flags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags(c_int);

impl OpenFlags {
    /// `SQLITE_OPEN_READONLY`
    pub const SQLITE_OPEN_READ_ONLY: OpenFlags = OpenFlags(ffi::SQLITE_OPEN_READONLY);
    /// `SQLITE_OPEN_READWRITE`
    pub const SQLITE_OPEN_READ_WRITE: OpenFlags = OpenFlags(ffi::SQLITE_OPEN_READWRITE);
    /// `SQLITE_OPEN_CREATE`
    pub const SQLITE_OPEN_CREATE: OpenFlags = OpenFlags(ffi::SQLITE_OPEN_CREATE);
    /// `SQLITE_OPEN_URI`
    pub const SQLITE_OPEN_URI: OpenFlags = OpenFlags(ffi::SQLITE_OPEN_URI);
    /// `SQLITE_OPEN_NOMUTEX`
    pub const SQLITE_OPEN_NO_MUTEX: OpenFlags = OpenFlags(ffi::SQLITE_OPEN_NOMUTEX);
    /// `SQLITE_OPEN_FULLMUTEX`
    pub const SQLITE_OPEN_FULL_MUTEX: OpenFlags = OpenFlags(ffi::SQLITE_OPEN_FULLMUTEX);
}

impl BitOr for OpenFlags {
    type Output = OpenFlags;
    fn bitor(self, o: OpenFlags) -> OpenFlags {
        OpenFlags(self.0 | o.0)
    }
}

impl Default for OpenFlags {
    fn default() -> Self {
        Self::SQLITE_OPEN_READ_WRITE
            | Self::SQLITE_OPEN_CREATE
            | Self::SQLITE_OPEN_URI
            | Self::SQLITE_OPEN_NO_MUTEX
    }
}

/// Flags for [`Connection::create_scalar_function`]. UTF-8 is always added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionFlags(c_int);

impl FunctionFlags {
    /// `SQLITE_UTF8`
    pub const SQLITE_UTF8: FunctionFlags = FunctionFlags(ffi::SQLITE_UTF8);
    /// `SQLITE_DETERMINISTIC`
    pub const SQLITE_DETERMINISTIC: FunctionFlags = FunctionFlags(ffi::SQLITE_DETERMINISTIC);
}

impl BitOr for FunctionFlags {
    type Output = FunctionFlags;
    fn bitor(self, o: FunctionFlags) -> FunctionFlags {
        FunctionFlags(self.0 | o.0)
    }
}

/// How [`Connection::transaction_with_behavior`] begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionBehavior {
    /// `BEGIN DEFERRED`
    Deferred,
    /// `BEGIN IMMEDIATE`
    Immediate,
    /// `BEGIN EXCLUSIVE`
    Exclusive,
}

struct RawStmt(*mut ffi::sqlite3_stmt);

/// A SQLite connection: owned (opened here, closed on drop) or borrowed from a host
/// (extension entry point, auto-extension). `Send`, not `Sync`: use one per thread.
pub struct Connection {
    db: *mut ffi::sqlite3,
    owned: bool,
    cache: RefCell<HashMap<String, Vec<RawStmt>>>,
}

// SAFETY: a connection may move between threads; it is !Sync (RefCell), so it is never used
// from two threads at once, which is all SQLITE_OPEN_NOMUTEX requires.
unsafe impl Send for Connection {}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("path", &self.path())
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Open read-write, creating the file (`READ_WRITE | CREATE | URI | NO_MUTEX`).
    pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
        Self::open_with_flags(path, OpenFlags::default())
    }

    /// An in-memory database.
    pub fn open_in_memory() -> Result<Connection> {
        Self::open(":memory:")
    }

    /// Open with explicit flags.
    pub fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> Result<Connection> {
        let p = path.as_ref().to_str().ok_or_else(|| Error::Sqlite {
            code: ffi::SQLITE_CANTOPEN,
            message: "path is not UTF-8".into(),
        })?;
        let c = cstring(p)?;
        let mut db = std::ptr::null_mut();
        // SAFETY: valid C string and out pointer; on failure the handle (if any) is closed.
        let rc = unsafe { ffi::sqlite3_open_v2(c.as_ptr(), &mut db, flags.0, std::ptr::null()) };
        if rc != ffi::SQLITE_OK {
            let e = db_error(db, rc);
            // SAFETY: sqlite3_close accepts null and a failed-open handle.
            unsafe { ffi::sqlite3_close(db) };
            return Err(e);
        }
        // SAFETY: live handle; extended codes make errors precise.
        unsafe { ffi::sqlite3_extended_result_codes(db, 1) };
        Ok(Connection {
            db,
            owned: true,
            cache: RefCell::default(),
        })
    }

    /// # Safety
    /// `db` must be a live handle that outlives the returned value.
    pub(crate) unsafe fn borrowed(db: *mut ffi::sqlite3) -> Connection {
        Connection {
            db,
            owned: false,
            cache: RefCell::default(),
        }
    }

    /// The `main` database file, `Some("")` for an in-memory database.
    pub fn path(&self) -> Option<String> {
        // SAFETY: live handle; the returned string is copied immediately.
        unsafe {
            let p = ffi::sqlite3_db_filename(self.db, c"main".as_ptr());
            (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    }

    /// Run one or more `;`-separated statements without parameters.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        let c = cstring(sql)?;
        let mut err: *mut c_char = std::ptr::null_mut();
        // SAFETY: valid handle and SQL; `err` is freed with sqlite3_free.
        let rc =
            unsafe { ffi::sqlite3_exec(self.db, c.as_ptr(), None, std::ptr::null_mut(), &mut err) };
        if rc == ffi::SQLITE_OK {
            return Ok(());
        }
        let message = if err.is_null() {
            return Err(db_error(self.db, rc));
        } else {
            // SAFETY: err is a NUL-terminated string from SQLite, freed once.
            unsafe {
                let m = CStr::from_ptr(err).to_string_lossy().into_owned();
                ffi::sqlite3_free(err as *mut c_void);
                m
            }
        };
        // SAFETY: live handle.
        Err(Error::Sqlite {
            code: unsafe { ffi::sqlite3_extended_errcode(self.db) },
            message,
        })
    }

    /// Prepare one statement.
    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>> {
        let n = len_c(sql.len())?;
        let mut st = std::ptr::null_mut();
        // SAFETY: sql pointer/length are valid for the call; SQLite copies what it needs.
        let rc = unsafe {
            ffi::sqlite3_prepare_v2(
                self.db,
                sql.as_ptr() as *const c_char,
                n,
                &mut st,
                std::ptr::null_mut(),
            )
        };
        if rc != ffi::SQLITE_OK {
            return Err(db_error(self.db, rc));
        }
        if st.is_null() {
            return Err(Error::Sqlite {
                code: ffi::SQLITE_MISUSE,
                message: "empty statement".into(),
            });
        }
        Ok(Statement {
            raw: st,
            db: self.db,
            _c: PhantomData,
        })
    }

    /// Prepare through a per-connection cache keyed by the SQL text.
    pub fn prepare_cached(&self, sql: &str) -> Result<CachedStatement<'_>> {
        let hit = self.cache.borrow_mut().get_mut(sql).and_then(Vec::pop);
        let stmt = match hit {
            Some(r) => Statement {
                raw: r.0,
                db: self.db,
                _c: PhantomData,
            },
            None => self.prepare(sql)?,
        };
        Ok(CachedStatement {
            conn: self,
            sql: sql.to_owned(),
            stmt: Some(stmt),
        })
    }

    /// Prepare, bind, run to completion; returns `sqlite3_changes`.
    pub fn execute(&self, sql: &str, params: impl Params) -> Result<usize> {
        self.prepare(sql)?.execute(params)
    }

    /// First row of a query, or [`Error::QueryReturnedNoRows`].
    pub fn query_row<T>(
        &self,
        sql: &str,
        params: impl Params,
        f: impl FnOnce(&Row<'_>) -> Result<T>,
    ) -> Result<T> {
        self.prepare(sql)?.query_row(params, f)
    }

    /// Rowid of the last insert on this connection.
    pub fn last_insert_rowid(&self) -> i64 {
        // SAFETY: live handle.
        unsafe { ffi::sqlite3_last_insert_rowid(self.db) }
    }

    /// Begin a transaction; it rolls back on drop unless committed.
    pub fn transaction_with_behavior(&mut self, b: TransactionBehavior) -> Result<Transaction<'_>> {
        self.execute_batch(match b {
            TransactionBehavior::Deferred => "BEGIN DEFERRED",
            TransactionBehavior::Immediate => "BEGIN IMMEDIATE",
            TransactionBehavior::Exclusive => "BEGIN EXCLUSIVE",
        })?;
        Ok(Transaction {
            conn: self,
            done: false,
        })
    }

    /// Register a scalar function. Arguments are copied into a [`Context`].
    pub fn create_scalar_function<F, T>(
        &self,
        name: &str,
        n_arg: c_int,
        flags: FunctionFlags,
        f: F,
    ) -> Result<()>
    where
        F: Fn(&Context) -> Result<T> + Send + 'static,
        T: ToSql,
    {
        type Boxed = Box<dyn Fn(&Context) -> Result<Value> + Send>;
        unsafe extern "C" fn call(
            ctx: *mut ffi::sqlite3_context,
            argc: c_int,
            argv: *mut *mut ffi::sqlite3_value,
        ) {
            let r = catch_unwind(AssertUnwindSafe(|| {
                // SAFETY: user data is the Boxed we registered; argv has argc live values.
                let (f, args) = unsafe {
                    let f = &*(ffi::sqlite3_user_data(ctx) as *const Boxed);
                    let args = (0..argc.max(0) as usize)
                        .map(|i| value_of(*argv.add(i)))
                        .collect();
                    (f, args)
                };
                f(&Context { args })
            }));
            // SAFETY: ctx is live for this call.
            unsafe {
                match r {
                    Ok(Ok(v)) => set_result(ctx, &v),
                    Ok(Err(e)) => set_error(ctx, &e.to_string()),
                    Err(_) => set_error(ctx, "panic in sqlite-csr-graph function"),
                }
            }
        }
        unsafe extern "C" fn destroy(p: *mut c_void) {
            // SAFETY: p is the Box we leaked below; SQLite calls this exactly once.
            let _ = catch_unwind(|| drop(unsafe { Box::from_raw(p as *mut Boxed) }));
        }
        let name = cstring(name)?;
        let boxed: Box<Boxed> = Box::new(Box::new(move |c: &Context| f(c).map(|v| v.to_sql())));
        let p = Box::into_raw(boxed) as *mut c_void;
        // SAFETY: SQLite owns `p` from here and calls `destroy` (also on failure).
        let rc = unsafe {
            ffi::sqlite3_create_function_v2(
                self.db,
                name.as_ptr(),
                n_arg,
                flags.0 | ffi::SQLITE_UTF8,
                p,
                Some(call),
                None,
                None,
                Some(destroy),
            )
        };
        if rc != ffi::SQLITE_OK {
            Err(db_error(self.db, rc))
        } else {
            Ok(())
        }
    }

    /// Register an eponymous-only table-valued function.
    pub(crate) fn create_table_function(
        &self,
        name: &CStr,
        f: Box<dyn TableFunction>,
    ) -> Result<()> {
        unsafe extern "C" fn destroy(p: *mut c_void) {
            // SAFETY: p is the Box<Box<dyn TableFunction>> leaked below; called once.
            let _ =
                catch_unwind(|| drop(unsafe { Box::from_raw(p as *mut Box<dyn TableFunction>) }));
        }
        let p = Box::into_raw(Box::new(f)) as *mut c_void;
        // SAFETY: MODULE is 'static; SQLite owns `p` and calls `destroy` (also on failure).
        let rc = unsafe {
            ffi::sqlite3_create_module_v2(self.db, name.as_ptr(), &MODULE, p, Some(destroy))
        };
        if rc != ffi::SQLITE_OK {
            Err(db_error(self.db, rc))
        } else {
            Ok(())
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        for (_, v) in self.cache.get_mut().drain() {
            for s in v {
                // SAFETY: cached statements belong to this handle and are finalized once.
                unsafe { ffi::sqlite3_finalize(s.0) };
            }
        }
        if self.owned {
            // SAFETY: we opened it; every statement borrows the connection, so none is still
            // open, and the cached ones were finalized above.
            unsafe { ffi::sqlite3_close(self.db) };
        }
    }
}

/// Arguments of a scalar function call.
#[derive(Debug)]
pub struct Context {
    args: Vec<Value>,
}

impl Context {
    /// Argument `i`, converted.
    pub fn get<T: FromSql>(&self, i: usize) -> Result<T> {
        T::from_sql(self.args.get(i).cloned().unwrap_or(Value::Null), i)
    }
    /// Number of arguments.
    pub fn len(&self) -> usize {
        self.args.len()
    }
    /// No arguments.
    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }
}

/// A transaction; derefs to its [`Connection`]. Rolls back on drop unless committed.
#[derive(Debug)]
pub struct Transaction<'c> {
    conn: &'c Connection,
    done: bool,
}

impl Transaction<'_> {
    /// `COMMIT`
    pub fn commit(mut self) -> Result<()> {
        self.done = true;
        self.conn.execute_batch("COMMIT")
    }
}

impl Deref for Transaction<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

// ---------------------------------------------------------------- statements

/// A prepared statement.
pub struct Statement<'c> {
    raw: *mut ffi::sqlite3_stmt,
    db: *mut ffi::sqlite3,
    _c: PhantomData<&'c Connection>,
}

impl std::fmt::Debug for Statement<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Statement").finish_non_exhaustive()
    }
}

impl Drop for Statement<'_> {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: owned statement, finalized once.
            unsafe { ffi::sqlite3_finalize(self.raw) };
        }
    }
}

impl<'c> Statement<'c> {
    /// Number of result columns.
    pub fn column_count(&self) -> usize {
        // SAFETY: live statement.
        unsafe { ffi::sqlite3_column_count(self.raw) as usize }
    }

    fn bind(&mut self, params: impl Params) -> Result<()> {
        // SAFETY: live statement; text and blobs are bound SQLITE_TRANSIENT (copied).
        unsafe {
            ffi::sqlite3_reset(self.raw);
            ffi::sqlite3_clear_bindings(self.raw);
        }
        for (i, v) in params.values().iter().enumerate() {
            let i = (i + 1) as c_int;
            // SAFETY: as above.
            let rc = unsafe {
                match v {
                    Value::Null => ffi::sqlite3_bind_null(self.raw, i),
                    Value::Integer(n) => ffi::sqlite3_bind_int64(self.raw, i, *n),
                    Value::Real(x) => ffi::sqlite3_bind_double(self.raw, i, *x),
                    Value::Text(s) => ffi::sqlite3_bind_text(
                        self.raw,
                        i,
                        s.as_ptr() as *const c_char,
                        len_c(s.len())?,
                        ffi::SQLITE_TRANSIENT(),
                    ),
                    Value::Blob(b) => ffi::sqlite3_bind_blob(
                        self.raw,
                        i,
                        b.as_ptr() as *const c_void,
                        len_c(b.len())?,
                        ffi::SQLITE_TRANSIENT(),
                    ),
                }
            };
            if rc != ffi::SQLITE_OK {
                return Err(db_error(self.db, rc));
            }
        }
        Ok(())
    }

    fn step(&mut self) -> Result<bool> {
        // SAFETY: live statement.
        match unsafe { ffi::sqlite3_step(self.raw) } {
            ffi::SQLITE_ROW => Ok(true),
            ffi::SQLITE_DONE => Ok(false),
            rc => Err(db_error(self.db, rc)),
        }
    }

    fn reset(&mut self) {
        // SAFETY: live statement; the error (already reported by step) is ignored.
        unsafe { ffi::sqlite3_reset(self.raw) };
    }

    /// Bind and run to completion; returns `sqlite3_changes`.
    pub fn execute(&mut self, params: impl Params) -> Result<usize> {
        self.bind(params)?;
        let r = (|| {
            while self.step()? {}
            Ok(())
        })();
        self.reset();
        r?;
        // SAFETY: live handle.
        Ok(unsafe { ffi::sqlite3_changes(self.db) } as usize)
    }

    /// Bind and iterate rows with [`Rows::next`].
    pub fn query(&mut self, params: impl Params) -> Result<Rows<'_, 'c>> {
        self.bind(params)?;
        Ok(Rows { stmt: self })
    }

    /// Bind, read every row through `f`, and return them.
    pub fn query_map<T>(
        &mut self,
        params: impl Params,
        mut f: impl FnMut(&Row<'_>) -> Result<T>,
    ) -> Result<std::vec::IntoIter<Result<T>>> {
        let mut out = vec![];
        let mut rows = self.query(params)?;
        while let Some(r) = rows.next()? {
            out.push(f(&r));
        }
        Ok(out.into_iter())
    }

    /// First row, or [`Error::QueryReturnedNoRows`].
    pub fn query_row<T>(
        &mut self,
        params: impl Params,
        f: impl FnOnce(&Row<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut rows = self.query(params)?;
        match rows.next()? {
            Some(r) => f(&r),
            None => Err(Error::QueryReturnedNoRows),
        }
    }
}

/// A statement from [`Connection::prepare_cached`]; returns to the cache on drop.
pub struct CachedStatement<'c> {
    conn: &'c Connection,
    sql: String,
    stmt: Option<Statement<'c>>,
}

impl std::fmt::Debug for CachedStatement<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedStatement")
            .field("sql", &self.sql)
            .finish_non_exhaustive()
    }
}

impl<'c> Deref for CachedStatement<'c> {
    type Target = Statement<'c>;
    fn deref(&self) -> &Statement<'c> {
        self.stmt.as_ref().expect("statement present until drop")
    }
}
impl DerefMut for CachedStatement<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.stmt.as_mut().expect("statement present until drop")
    }
}

impl Drop for CachedStatement<'_> {
    fn drop(&mut self) {
        if let Some(mut s) = self.stmt.take() {
            s.reset();
            let raw = std::mem::replace(&mut s.raw, std::ptr::null_mut());
            self.conn
                .cache
                .borrow_mut()
                .entry(std::mem::take(&mut self.sql))
                .or_default()
                .push(RawStmt(raw));
        }
    }
}

/// Rows of a running query. The statement is reset when this is dropped.
#[derive(Debug)]
pub struct Rows<'s, 'c> {
    stmt: &'s mut Statement<'c>,
}

impl Rows<'_, '_> {
    /// Advance; `None` when done.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Row<'_>>> {
        Ok(self.stmt.step()?.then_some(Row {
            raw: self.stmt.raw,
            _r: PhantomData,
        }))
    }
}

impl Drop for Rows<'_, '_> {
    fn drop(&mut self) {
        self.stmt.reset();
    }
}

/// The current row.
#[derive(Debug)]
pub struct Row<'r> {
    raw: *mut ffi::sqlite3_stmt,
    _r: PhantomData<&'r ()>,
}

/// Column index type for [`Row::get`].
pub trait RowIndex {
    /// Zero-based index.
    fn index(&self) -> usize;
}
impl RowIndex for usize {
    fn index(&self) -> usize {
        *self
    }
}

impl Row<'_> {
    /// Column `i`, converted.
    pub fn get<I: RowIndex, T: FromSql>(&self, i: I) -> Result<T> {
        let i = i.index();
        // SAFETY: live statement positioned on a row (Row borrows Rows, which holds it there).
        let n = unsafe { ffi::sqlite3_column_count(self.raw) } as usize;
        if i >= n {
            return bad(i);
        }
        // SAFETY: as above; pointers are copied before the next column call.
        let v = unsafe {
            let c = i as c_int;
            match ffi::sqlite3_column_type(self.raw, c) {
                ffi::SQLITE_INTEGER => Value::Integer(ffi::sqlite3_column_int64(self.raw, c)),
                ffi::SQLITE_FLOAT => Value::Real(ffi::sqlite3_column_double(self.raw, c)),
                ffi::SQLITE_TEXT => {
                    let p = ffi::sqlite3_column_text(self.raw, c);
                    let len = ffi::sqlite3_column_bytes(self.raw, c) as usize;
                    Value::Text(if p.is_null() {
                        String::new()
                    } else {
                        String::from_utf8_lossy(std::slice::from_raw_parts(p, len)).into_owned()
                    })
                }
                ffi::SQLITE_BLOB => {
                    let p = ffi::sqlite3_column_blob(self.raw, c) as *const u8;
                    let len = ffi::sqlite3_column_bytes(self.raw, c) as usize;
                    Value::Blob(if p.is_null() {
                        vec![]
                    } else {
                        std::slice::from_raw_parts(p, len).to_vec()
                    })
                }
                _ => Value::Null,
            }
        };
        T::from_sql(v, i)
    }
}

// ---------------------------------------------------------------- table-valued functions

/// An eponymous table-valued function: hidden argument columns follow the `ncols` result columns.
pub(crate) trait TableFunction: Send + Sync + 'static {
    fn schema(&self) -> &'static CStr;
    fn ncols(&self) -> c_int;
    fn nargs(&self) -> c_int;
    fn run(&self, args: &[Value]) -> Result<Vec<Vec<Value>>>;
}

#[repr(C)]
struct Tab {
    base: ffi::sqlite3_vtab,
    func: *const Box<dyn TableFunction>,
}

#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    rows: Vec<Vec<Value>>,
    pos: usize,
}

/// Eponymous-only (`xCreate` null): usable as `graph_impact(...)` without `CREATE VIRTUAL TABLE`.
static MODULE: ffi::sqlite3_module = ffi::sqlite3_module {
    iVersion: 0,
    xCreate: None,
    xConnect: Some(x_connect),
    xBestIndex: Some(x_best_index),
    xDisconnect: Some(x_disconnect),
    xDestroy: Some(x_disconnect),
    xOpen: Some(x_open),
    xClose: Some(x_close),
    xFilter: Some(x_filter),
    xNext: Some(x_next),
    xEof: Some(x_eof),
    xColumn: Some(x_column),
    xRowid: Some(x_rowid),
    // SAFETY: every remaining field is an optional callback; all-zero is None.
    ..unsafe { std::mem::zeroed() }
};

fn guard(f: impl FnOnce() -> c_int) -> c_int {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(ffi::SQLITE_ERROR)
}

/// # Safety
/// `vtab` must be a live `Tab`.
unsafe fn set_vtab_error(vtab: *mut ffi::sqlite3_vtab, msg: &str) {
    // SAFETY: zErrMsg is owned by SQLite memory (freed with sqlite3_free).
    unsafe {
        if !(*vtab).zErrMsg.is_null() {
            ffi::sqlite3_free((*vtab).zErrMsg as *mut c_void);
        }
        (*vtab).zErrMsg = sqlite_string(msg);
    }
}

unsafe extern "C" fn x_connect(
    db: *mut ffi::sqlite3,
    aux: *mut c_void,
    _argc: c_int,
    _argv: *const *const c_char,
    pp: *mut *mut ffi::sqlite3_vtab,
    _err: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: aux is the Box<dyn TableFunction> registered with the module; it outlives
        // every vtab (SQLite destroys aux only after disconnecting them).
        unsafe {
            let func = aux as *const Box<dyn TableFunction>;
            let rc = ffi::sqlite3_declare_vtab(db, (*func).schema().as_ptr());
            if rc != ffi::SQLITE_OK {
                return rc;
            }
            ffi::sqlite3_vtab_config(db, ffi::SQLITE_VTAB_INNOCUOUS);
            *pp = Box::into_raw(Box::new(Tab {
                base: ffi::sqlite3_vtab::default(),
                func,
            })) as *mut ffi::sqlite3_vtab;
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_disconnect(vtab: *mut ffi::sqlite3_vtab) -> c_int {
    // SAFETY: vtab came from Box<Tab> in x_connect; SQLite disconnects once.
    guard(|| {
        drop(unsafe { Box::from_raw(vtab as *mut Tab) });
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_best_index(
    vtab: *mut ffi::sqlite3_vtab,
    info: *mut ffi::sqlite3_index_info,
) -> c_int {
    guard(|| {
        // SAFETY: SQLite passes a live index_info whose arrays have nConstraint entries.
        unsafe {
            let f = &**(*(vtab as *mut Tab)).func;
            let info = &mut *info;
            let (ncols, nargs) = (f.ncols(), f.nargs());
            let mut slot: Vec<Option<usize>> = vec![None; nargs as usize];
            for i in 0..info.nConstraint.max(0) as usize {
                let c = &*info.aConstraint.add(i);
                let col = c.iColumn - ncols;
                if col < 0 || col >= nargs || c.op as c_int != ffi::SQLITE_INDEX_CONSTRAINT_EQ {
                    continue;
                }
                if c.usable == 0 {
                    return ffi::SQLITE_CONSTRAINT;
                }
                slot[col as usize] = Some(i);
            }
            let (mut mask, mut n) = (0, 0);
            for (a, s) in slot.iter().enumerate() {
                if let Some(i) = s {
                    n += 1;
                    mask |= 1 << a;
                    let u = &mut *info.aConstraintUsage.add(*i);
                    u.argvIndex = n;
                    u.omit = 1;
                }
            }
            info.idxNum = mask;
            info.estimatedCost = if mask & 1 != 0 { 10.0 } else { 1e12 };
            info.estimatedRows = 100;
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_open(
    _vtab: *mut ffi::sqlite3_vtab,
    pp: *mut *mut ffi::sqlite3_vtab_cursor,
) -> c_int {
    guard(|| {
        let c = Box::new(Cursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            rows: vec![],
            pos: 0,
        });
        // SAFETY: out pointer from SQLite; freed in x_close.
        unsafe { *pp = Box::into_raw(c) as *mut ffi::sqlite3_vtab_cursor };
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_close(cur: *mut ffi::sqlite3_vtab_cursor) -> c_int {
    // SAFETY: cur came from Box<Cursor> in x_open; closed once.
    guard(|| {
        drop(unsafe { Box::from_raw(cur as *mut Cursor) });
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_filter(
    cur: *mut ffi::sqlite3_vtab_cursor,
    idx_num: c_int,
    _idx_str: *const c_char,
    argc: c_int,
    argv: *mut *mut ffi::sqlite3_value,
) -> c_int {
    let r = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: cur is our Cursor, its pVtab our Tab; argv has argc live values.
        unsafe {
            let c = &mut *(cur as *mut Cursor);
            let f = &**(*(c.base.pVtab as *mut Tab)).func;
            let mut a = vec![Value::Null; f.nargs() as usize];
            let mut j = 0;
            for (i, slot) in a.iter_mut().enumerate() {
                if idx_num & (1 << i) != 0 && j < argc {
                    *slot = value_of(*argv.add(j as usize));
                    j += 1;
                }
            }
            let rows = f.run(&a)?;
            c.rows = rows;
            c.pos = 0;
            Ok::<_, Error>(())
        }
    }));
    let msg = match r {
        Ok(Ok(())) => return ffi::SQLITE_OK,
        Ok(Err(e)) => e.to_string(),
        Err(_) => "panic in sqlite-csr-graph table function".into(),
    };
    // SAFETY: as above.
    unsafe { set_vtab_error((*cur).pVtab, &msg) };
    ffi::SQLITE_ERROR
}

unsafe extern "C" fn x_next(cur: *mut ffi::sqlite3_vtab_cursor) -> c_int {
    // SAFETY: cur is our Cursor.
    guard(|| {
        unsafe { (*(cur as *mut Cursor)).pos += 1 };
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_eof(cur: *mut ffi::sqlite3_vtab_cursor) -> c_int {
    // SAFETY: cur is our Cursor.
    catch_unwind(|| {
        let c = unsafe { &*(cur as *const Cursor) };
        (c.pos >= c.rows.len()) as c_int
    })
    .unwrap_or(1)
}

unsafe extern "C" fn x_column(
    cur: *mut ffi::sqlite3_vtab_cursor,
    ctx: *mut ffi::sqlite3_context,
    i: c_int,
) -> c_int {
    guard(|| {
        // SAFETY: cur is our Cursor on a valid row (SQLite checks xEof first); ctx is live.
        unsafe {
            let c = &*(cur as *const Cursor);
            match c.rows.get(c.pos).and_then(|r| r.get(i as usize)) {
                Some(v) => set_result(ctx, v),
                None => ffi::sqlite3_result_null(ctx),
            }
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_rowid(
    cur: *mut ffi::sqlite3_vtab_cursor,
    out: *mut ffi::sqlite3_int64,
) -> c_int {
    guard(|| {
        // SAFETY: cur is our Cursor; out is SQLite's out pointer.
        unsafe { *out = (*(cur as *const Cursor)).pos as i64 };
        ffi::SQLITE_OK
    })
}

// ---------------------------------------------------------------- entry points

/// Register on a host-provided handle; errors go to `*err` (sqlite3_malloc'ed) if non-null.
///
/// # Safety
/// `db` must be live for the call; `err` null or writable.
unsafe fn init_on(db: *mut ffi::sqlite3, err: *mut *mut c_char, skip_memory: bool) -> c_int {
    guard(|| {
        // SAFETY: db is live for the call; the borrowed Connection does not close it.
        let conn = unsafe { Connection::borrowed(db) };
        if skip_memory && conn.path().is_none_or(|p| p.is_empty()) {
            return ffi::SQLITE_OK;
        }
        match crate::vtab::register(&conn) {
            Ok(()) => ffi::SQLITE_OK,
            Err(e) => {
                if !err.is_null() {
                    // SAFETY: err is writable; SQLite frees the message.
                    unsafe { *err = sqlite_string(&e.to_string()) };
                }
                ffi::SQLITE_ERROR
            }
        }
    })
}

/// SQLite loadable-extension entry point (`.load libsqlite_csr_graph` / `load_extension`).
/// Built with `--no-default-features --features loadable`.
///
/// # Safety
///
/// Called by SQLite only (`sqlite3_load_extension`), with the `db`, error-message and API
/// pointers SQLite passes to an extension entry point.
#[cfg(feature = "loadable")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite3_extension_init(
    db: *mut ffi::sqlite3,
    pz_err_msg: *mut *mut c_char,
    p_api: *mut ffi::sqlite3_api_routines,
) -> c_int {
    guard(|| {
        // SAFETY: p_api is SQLite's routine table, valid for the process (SQLITE_EXTENSION_INIT2).
        // On failure the API table is unusable, so no message can be allocated.
        if unsafe { ffi::rusqlite_extension_init2(p_api) }.is_err() {
            return ffi::SQLITE_ERROR;
        }
        // SAFETY: db/pz_err_msg as passed by SQLite.
        unsafe { init_on(db, pz_err_msg, false) }
    })
}

#[cfg(not(feature = "loadable"))]
unsafe extern "C" fn auto_init(
    db: *mut ffi::sqlite3,
    err: *mut *mut c_char,
    _api: *const ffi::sqlite3_api_routines,
) -> c_int {
    // SAFETY: called by SQLite for each new connection with a live handle.
    unsafe { init_on(db, err, true) }
}

/// Register the graph functions on every connection this process opens from now on
/// (`sqlite3_auto_extension`), including connections opened by other crates (diesel)
/// against the same SQLite. In-memory databases are skipped. Idempotent.
#[cfg(not(feature = "loadable"))]
pub fn register_auto_extension() -> Result<()> {
    // SAFETY: auto_init is a 'static function with the entry-point signature.
    let rc = unsafe { ffi::sqlite3_auto_extension(Some(auto_init)) };
    if rc == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(Error::Sqlite {
            code: rc,
            message: "sqlite3_auto_extension failed".into(),
        })
    }
}

/// Undo [`register_auto_extension`]; returns whether it was registered.
#[cfg(not(feature = "loadable"))]
pub fn cancel_auto_extension() -> bool {
    // SAFETY: same function pointer as registered.
    unsafe { ffi::sqlite3_cancel_auto_extension(Some(auto_init)) == 1 }
}

/// `sqlite3_config(SQLITE_CONFIG_MEMSTATUS, 0)`; false if SQLite is already initialised.
#[cfg(not(feature = "loadable"))]
pub(crate) fn disable_memstatus() -> bool {
    // SAFETY: sqlite3_config is only valid before sqlite3_initialize; SQLite returns
    // SQLITE_MISUSE (no effect) otherwise, which we report as false.
    unsafe { ffi::sqlite3_config(ffi::SQLITE_CONFIG_MEMSTATUS, 0 as c_int) == ffi::SQLITE_OK }
}
