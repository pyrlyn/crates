//! Plugin-side SDK for plugwright.
//!
//! Put [`#[plugin]`](plugin) on an `impl` block: every `&self`/`&mut self` method
//! becomes a plugin method, arguments and results are encoded for you, and the
//! ABI exports are emitted. The same crate builds as a native `cdylib`
//! (`.so`/`.dylib`/`.dll`), a `staticlib`, or — for `wasm32-unknown-unknown` —
//! a core Wasm module.
//!
//! ```ignore
//! #[derive(Default)]
//! struct Echo;
//!
//! #[plugwright_sdk::plugin]
//! impl Echo {
//!     fn echo(&self, input: &[u8]) -> Vec<u8> { input.to_vec() }
//!     fn add(&self, a: i64, b: i64) -> i64 { a + b }
//! }
//! ```
//!
//! Without the `macros` feature, implement [`Plugin`] by hand and call
//! [`export_plugin!`] once.

pub use plugwright_abi as abi;
#[cfg(feature = "macros")]
pub use plugwright_macros::plugin;
#[cfg(feature = "macros")]
pub use {serde, serde_json};

/// SDK version embedded into every plugin; the host checks it for compatibility.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The interface a plugin implements. Inputs and outputs are opaque bytes; the
/// host and the plugin agree on the encoding (JSON, bincode, text, …).
pub trait Plugin: Send + Sized + 'static {
    /// Build the plugin from the `config` table of its manifest entry (JSON).
    fn new(config: &[u8]) -> Result<Self, String>;
    /// Handle one call. An `Err` is reported to the host as a plugin error.
    fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>, String>;
}

/// Logging routed to the host's `tracing` subscriber: a function per level
/// taking `&str`, and a macro per level taking `format!` arguments
/// (`log::info!("loaded {n} rules")`).
pub mod log {
    use crate::abi;

    #[doc(hidden)]
    pub fn emit(level: u32, msg: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        crate::__native::emit_log(level, msg);
        #[cfg(target_arch = "wasm32")]
        crate::__wasm::emit_log(level, msg);
    }

    macro_rules! levels {
        ($($name:ident => $level:ident),* $(,)?) => {$(
            #[doc = concat!("Log `msg` at `", stringify!($level), "`.")]
            pub fn $name(msg: &str) {
                emit(abi::$level, msg)
            }
        )*};
    }

    levels! {
        error => LOG_ERROR,
        warn => LOG_WARN,
        info => LOG_INFO,
        debug => LOG_DEBUG,
        trace => LOG_TRACE,
    }

    pub use crate::{debug, error, info, trace, warn};
}

// `#[macro_export]` macros cannot themselves be macro-generated and still be
// re-exported by path, so these five stay spelled out.
/// `format!`-style [`log::error`].
#[macro_export]
macro_rules! error { ($($arg:tt)*) => { $crate::log::emit($crate::abi::LOG_ERROR, &::std::format!($($arg)*)) } }
/// `format!`-style [`log::warn`].
#[macro_export]
macro_rules! warn { ($($arg:tt)*) => { $crate::log::emit($crate::abi::LOG_WARN, &::std::format!($($arg)*)) } }
/// `format!`-style [`log::info`].
#[macro_export]
macro_rules! info { ($($arg:tt)*) => { $crate::log::emit($crate::abi::LOG_INFO, &::std::format!($($arg)*)) } }
/// `format!`-style [`log::debug`].
#[macro_export]
macro_rules! debug { ($($arg:tt)*) => { $crate::log::emit($crate::abi::LOG_DEBUG, &::std::format!($($arg)*)) } }
/// `format!`-style [`log::trace`].
#[macro_export]
macro_rules! trace { ($($arg:tt)*) => { $crate::log::emit($crate::abi::LOG_TRACE, &::std::format!($($arg)*)) } }

fn install_panic_logger() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // The plugin has its own copy of std, so this hook never touches the host's.
        std::panic::set_hook(Box::new(|info| log::error(&format!("plugin panicked: {info}"))));
    });
}

#[doc(hidden)]
#[cfg(not(target_arch = "wasm32"))]
pub mod __native {
    use crate::Plugin;
    use crate::abi::{PwBuf, PwHostApi, PwStatus, PwStr, STATUS_ERROR, STATUS_OK, STATUS_PANIC};
    use std::any::Any;
    use std::ffi::c_void;
    use std::mem::ManuallyDrop;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::OnceLock;

    struct Host(PwHostApi);
    // The host guarantees `ctx` stays valid and `log` is thread-safe.
    unsafe impl Send for Host {}
    unsafe impl Sync for Host {}

    static HOST: OnceLock<Host> = OnceLock::new();

    pub fn emit_log(level: u32, msg: &str) {
        if let Some(Host(api)) = HOST.get()
            && let Some(log) = api.log
        {
            unsafe { log(api.ctx, level, PwStr::from_bytes(msg.as_bytes())) }
        }
    }

    fn into_buf(v: Vec<u8>) -> PwBuf {
        let mut v = ManuallyDrop::new(v);
        PwBuf { ptr: v.as_mut_ptr(), len: v.len(), cap: v.capacity() }
    }

    pub unsafe extern "C" fn free_buf(b: PwBuf) {
        if !b.ptr.is_null() {
            drop(unsafe { Vec::from_raw_parts(b.ptr, b.len, b.cap) });
        }
    }

    fn panic_message(p: &(dyn Any + Send)) -> String {
        if let Some(s) = p.downcast_ref::<&str>() {
            (*s).to_owned()
        } else if let Some(s) = p.downcast_ref::<String>() {
            s.clone()
        } else {
            "non-string panic payload".to_owned()
        }
    }

    /// Run `f`, never letting a panic unwind into the host.
    fn guard(out: *mut PwBuf, f: impl FnOnce() -> Result<Vec<u8>, String>) -> PwStatus {
        let (status, bytes) = match catch_unwind(AssertUnwindSafe(f)) {
            Ok(Ok(v)) => (STATUS_OK, v),
            Ok(Err(e)) => (STATUS_ERROR, e.into_bytes()),
            Err(p) => (STATUS_PANIC, panic_message(p.as_ref()).into_bytes()),
        };
        if !out.is_null() {
            unsafe { out.write(into_buf(bytes)) };
        }
        status
    }

    pub unsafe extern "C" fn create<T: Plugin>(
        host: *const PwHostApi,
        config: PwStr,
        out_instance: *mut *mut c_void,
        out_err: *mut PwBuf,
    ) -> PwStatus {
        guard(out_err, || {
            if !host.is_null() {
                let _ = HOST.set(Host(unsafe { *host }));
            }
            super::install_panic_logger();
            if out_instance.is_null() {
                return Err("null out_instance".into());
            }
            let plugin = T::new(unsafe { config.as_bytes() })?;
            unsafe { *out_instance = Box::into_raw(Box::new(plugin)).cast() };
            Ok(Vec::new())
        })
    }

    pub unsafe extern "C" fn call<T: Plugin>(
        instance: *mut c_void,
        method: PwStr,
        input: PwStr,
        out: *mut PwBuf,
    ) -> PwStatus {
        guard(out, || {
            if instance.is_null() {
                return Err("null instance".into());
            }
            let plugin = unsafe { &mut *instance.cast::<T>() };
            let method =
                std::str::from_utf8(unsafe { method.as_bytes() }).map_err(|_| "method name is not UTF-8".to_owned())?;
            plugin.call(method, unsafe { input.as_bytes() })
        })
    }

    pub unsafe extern "C" fn destroy<T: Plugin>(instance: *mut c_void) {
        if !instance.is_null() {
            let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(instance.cast::<T>()) })));
        }
    }
}

#[doc(hidden)]
#[cfg(target_arch = "wasm32")]
pub mod __wasm {
    use crate::Plugin;
    use crate::abi::{STATUS_ERROR, STATUS_OK, wasm::pack};
    use std::alloc::{Layout, alloc as raw_alloc, dealloc};
    use std::any::Any;
    use std::cell::RefCell;

    #[link(wasm_import_module = "plugwright")]
    unsafe extern "C" {
        #[link_name = "log"]
        fn host_log(level: i32, ptr: i32, len: i32);
    }

    pub fn emit_log(level: u32, msg: &str) {
        unsafe { host_log(level as i32, msg.as_ptr() as i32, msg.len() as i32) }
    }

    thread_local! {
        static INSTANCE: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None) };
    }

    pub fn alloc(len: i32) -> i32 {
        if len <= 0 {
            return 1; // dangling, never dereferenced or freed
        }
        let layout = Layout::array::<u8>(len as usize).expect("allocation too large");
        let p = unsafe { raw_alloc(layout) };
        if p.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        p as i32
    }

    /// # Safety
    /// `(ptr, len)` must come from [`alloc`].
    pub unsafe fn free(ptr: i32, len: i32) {
        if len > 0 {
            unsafe { dealloc(ptr as *mut u8, Layout::array::<u8>(len as usize).unwrap()) }
        }
    }

    /// Copy a host-provided buffer and release it.
    unsafe fn take(ptr: i32, len: i32) -> Vec<u8> {
        if len <= 0 {
            return Vec::new();
        }
        let v = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }.to_vec();
        unsafe { free(ptr, len) };
        v
    }

    fn ret(status: u32, payload: &[u8]) -> i64 {
        let n = payload.len() + 1;
        let p = alloc(n as i32);
        unsafe {
            let dst = std::slice::from_raw_parts_mut(p as *mut u8, n);
            dst[0] = status as u8;
            dst[1..].copy_from_slice(payload);
        }
        pack(p as u32, n as u32)
    }

    pub fn meta(name: &str, version: &str) -> i64 {
        let s = format!("{name}\0{version}\0{}", crate::SDK_VERSION);
        let p = alloc(s.len() as i32);
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), p as *mut u8, s.len()) };
        pack(p as u32, s.len() as u32)
    }

    /// # Safety
    /// The buffer must come from [`alloc`]; ownership passes to the guest.
    pub unsafe fn init<T: Plugin>(ptr: i32, len: i32) -> i64 {
        super::install_panic_logger();
        let config = unsafe { take(ptr, len) };
        match T::new(&config) {
            Ok(p) => {
                INSTANCE.with(|i| *i.borrow_mut() = Some(Box::new(p)));
                ret(STATUS_OK, &[])
            }
            Err(e) => ret(STATUS_ERROR, e.as_bytes()),
        }
    }

    /// # Safety
    /// Both buffers must come from [`alloc`]; ownership passes to the guest.
    pub unsafe fn call<T: Plugin>(mptr: i32, mlen: i32, iptr: i32, ilen: i32) -> i64 {
        let method = unsafe { take(mptr, mlen) };
        let input = unsafe { take(iptr, ilen) };
        let Ok(method) = std::str::from_utf8(&method) else {
            return ret(STATUS_ERROR, b"method name is not UTF-8");
        };
        INSTANCE.with(|i| {
            let mut slot = i.borrow_mut();
            let Some(p) = slot.as_mut().and_then(|b| b.downcast_mut::<T>()) else {
                return ret(STATUS_ERROR, b"plugin is not initialized");
            };
            match p.call(method, &input) {
                Ok(v) => ret(STATUS_OK, &v),
                Err(e) => ret(STATUS_ERROR, e.as_bytes()),
            }
        })
    }
}

/// Export a [`Plugin`] implementation with the ABI matching the build target.
#[macro_export]
macro_rules! export_plugin {
    ($ty:ty) => {
        #[cfg(not(target_arch = "wasm32"))]
        const _: () = {
            static DECL: $crate::abi::PwPluginDecl = $crate::abi::PwPluginDecl {
                abi_version: $crate::abi::ABI_VERSION,
                struct_size: ::core::mem::size_of::<$crate::abi::PwPluginDecl>() as u32,
                sdk_version: $crate::abi::PwStr::from_static($crate::SDK_VERSION),
                name: $crate::abi::PwStr::from_static(env!("CARGO_PKG_NAME")),
                version: $crate::abi::PwStr::from_static(env!("CARGO_PKG_VERSION")),
                create: Some($crate::__native::create::<$ty>),
                call: Some($crate::__native::call::<$ty>),
                destroy: Some($crate::__native::destroy::<$ty>),
                free_buf: Some($crate::__native::free_buf),
            };

            #[unsafe(no_mangle)]
            pub extern "C" fn plugwright_entry_v1() -> *const $crate::abi::PwPluginDecl {
                &DECL
            }
        };

        #[cfg(target_arch = "wasm32")]
        const _: () = {
            #[unsafe(no_mangle)]
            pub extern "C" fn pw_abi_version() -> i32 {
                $crate::abi::ABI_VERSION as i32
            }
            #[unsafe(no_mangle)]
            pub extern "C" fn pw_alloc(len: i32) -> i32 {
                $crate::__wasm::alloc(len)
            }
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pw_free(ptr: i32, len: i32) {
                unsafe { $crate::__wasm::free(ptr, len) }
            }
            #[unsafe(no_mangle)]
            pub extern "C" fn pw_meta() -> i64 {
                $crate::__wasm::meta(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
            }
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pw_init(ptr: i32, len: i32) -> i64 {
                unsafe { $crate::__wasm::init::<$ty>(ptr, len) }
            }
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pw_call(mptr: i32, mlen: i32, iptr: i32, ilen: i32) -> i64 {
                unsafe { $crate::__wasm::call::<$ty>(mptr, mlen, iptr, ilen) }
            }
        };
    };
}

/// Runtime support for `#[plugin]`; not a public API.
#[doc(hidden)]
#[cfg(feature = "macros")]
pub mod __private {
    use serde::{Serialize, de::DeserializeOwned};
    use serde_json::{Map, Value};

    pub fn utf8<'a>(input: &'a [u8], what: &str) -> Result<&'a str, String> {
        std::str::from_utf8(input).map_err(|e| format!("`{what}` must be UTF-8: {e}"))
    }

    pub fn json<T: DeserializeOwned>(input: &[u8], what: &str) -> Result<T, String> {
        serde_json::from_slice(input).map_err(|e| format!("cannot decode `{what}`: {e}"))
    }

    pub fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, String> {
        serde_json::to_vec(value).map_err(|e| format!("cannot encode the result: {e}"))
    }

    pub fn unknown_method(method: &str, known: &[&str]) -> String {
        if known.is_empty() {
            format!("unknown method `{method}`")
        } else {
            format!("unknown method `{method}` (available: {})", known.join(", "))
        }
    }

    /// Arguments of a multi-parameter method: a JSON array or object.
    pub enum Args {
        Positional(Vec<Value>),
        Named(Map<String, Value>),
    }

    impl Args {
        pub fn parse(input: &[u8], arity: usize) -> Result<Self, String> {
            let shape = |got: &str| format!("expected {arity} arguments as a JSON array or object, got {got}");
            match serde_json::from_slice(input).map_err(|e| shape(&e.to_string()))? {
                Value::Array(a) if a.len() <= arity => Ok(Args::Positional(a)),
                Value::Array(a) => Err(shape(&format!("{} values", a.len()))),
                Value::Object(m) => Ok(Args::Named(m)),
                _ => Err(shape("a scalar")),
            }
        }

        /// A missing argument reads as `null`, so `Option<T>` parameters may be omitted.
        pub fn take<T: DeserializeOwned>(&mut self, index: usize, name: &str) -> Result<T, String> {
            let v = match self {
                Args::Positional(a) => a.get_mut(index).map(Value::take),
                Args::Named(m) => m.remove(name),
            };
            serde_json::from_value(v.unwrap_or(Value::Null)).map_err(|e| format!("argument `{name}`: {e}"))
        }

        /// Reject named arguments the method does not have.
        pub fn finish(self) -> Result<(), String> {
            match self {
                Args::Named(m) if !m.is_empty() => {
                    let names: Vec<&str> = m.keys().map(String::as_str).collect();
                    Err(format!("unknown argument(s): {}", names.join(", ")))
                }
                _ => Ok(()),
            }
        }
    }
}
