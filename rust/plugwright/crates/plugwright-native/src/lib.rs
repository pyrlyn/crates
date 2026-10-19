//! In-process loader for native plugins (`.so` / `.dylib` / `.dll`).

pub mod format;

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use libloading::Library;
use plugwright_abi::{
    ABI_VERSION, EntryFn, MAX_DECL_STR, NATIVE_ENTRY, PwBuf, PwHostApi, PwPluginDecl, PwStatus, PwStr, STATUS_ERROR,
    STATUS_OK, STATUS_PANIC,
};

use format::Format;
use plugwright_core::error::{PluginError, Result, io_err};
use plugwright_core::logging::LogSink;
use plugwright_core::plugin::{Backend, Plugin, PluginInfo, Release, check_sdk, parse_version};

/// Payloads larger than this coming back from a plugin are rejected.
pub const MAX_OUTPUT: usize = 256 << 20;
/// Guest log lines are truncated to this many bytes.
pub const MAX_LOG_LINE: usize = 64 << 10;

/// Per-load host context handed to the plugin as `PwHostApi::ctx`.
///
/// Deliberately leaked: a plugin may keep the pointer in a static and log from
/// its own threads after `destroy`, so it must never dangle. One small
/// allocation per load is the price.
struct HostCtx {
    sink: LogSink,
}

unsafe extern "C" fn host_log(ctx: *mut c_void, level: u32, msg: PwStr) {
    if ctx.is_null() {
        return;
    }
    // Never let a panicking subscriber unwind into plugin code.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let ctx = unsafe { &*ctx.cast::<HostCtx>() };
        let bytes = unsafe { msg.as_bytes() };
        let bytes = &bytes[..bytes.len().min(MAX_LOG_LINE)];
        (ctx.sink)(level, &String::from_utf8_lossy(bytes));
    }));
}

/// A native plugin instance living in this process.
///
/// Field order matters for `Drop`: the instance is destroyed first, then the
/// library is closed, then the shadow copy is removed.
pub struct NativePlugin {
    id: String,
    instance: *mut c_void,
    decl: *const PwPluginDecl,
    info: PluginInfo,
    lib: Option<Library>,
    /// The file actually handed to the loader.
    path: PathBuf,
    /// Private copy to delete on drop; reusable copies are kept.
    shadow: Option<PathBuf>,
}

// The SDK requires `Plugin: Send`; the decl table is immutable static data.
unsafe impl Send for NativePlugin {}

static LOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Which file the loader maps.
///
/// Every live instance needs an image of its own: the SDK keeps per-image
/// globals (the host callback table among them), and a library that could not
/// be unmapped stays in the process. So a load whose file is still mapped uses
/// a copy. Copies are named by content (or file identity), written once and
/// reused while unmapped, here and in later processes: the OS validates a file
/// the first time it maps it (macOS checks the code signature, ~90 ms).
#[derive(Debug, Clone, Copy)]
pub enum Placement<'a> {
    /// Map the file itself: no copy and no hashing. The file must not be
    /// rewritten while it is loaded (Windows also locks it). Copies, when
    /// needed, go to `copies` (the temp dir when `None`).
    InPlace { copies: Option<&'a Path> },
    /// Map a copy kept in `dir` under `tag` (the content hash). For files the
    /// user may rebuild in place while a plugin is loaded.
    Copy { dir: &'a Path, tag: &'a str },
}

/// Copies of one file that may exist side by side (live or leaked images).
const MAX_COPIES: usize = 64;

/// Remove the copies in `dir` that no loaded image uses; returns how many went.
/// A copy still mapped here (or locked by another process on Windows) stays.
pub fn prune_copies(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !still_mapped(p) && std::fs::remove_file(p).is_ok())
        .count()
}

/// The file to map for `placement`, plus a private copy to delete on drop.
fn image(id: &str, artifact: &Path, format: &Format, placement: Placement) -> Result<(PathBuf, Option<PathBuf>)> {
    let (dir, tag) = match placement {
        Placement::InPlace { copies } => {
            let path = std::fs::canonicalize(artifact).map_err(io_err(artifact))?;
            if !still_mapped(&path) {
                return Ok((path, None));
            }
            let dir = copies.map_or_else(|| std::env::temp_dir().join("plugwright-loaded"), Path::to_owned);
            (dir, identity_tag(&path)?)
        }
        Placement::Copy { dir, tag } => {
            let tag: String = tag.chars().filter(char::is_ascii_alphanumeric).take(32).collect();
            if tag.is_empty() {
                return Err(PluginError::Load("copy tag must contain letters or digits".into()));
            }
            (dir.to_owned(), tag)
        }
    };
    let ext = format::shared_ext(format);
    for n in 0..MAX_COPIES {
        let name = if n == 0 { format!("{tag}.{ext}") } else { format!("{tag}-{n}.{ext}") };
        let copy = dir.join(name);
        if !copy.is_file() {
            write_copy(artifact, &dir, &copy)?;
            return Ok((copy, None));
        }
        if !still_mapped(&copy) {
            return Ok((copy, None));
        }
    }
    let copy = shadow_copy(id, artifact, format, &dir)?;
    Ok((copy.clone(), Some(copy)))
}

/// Names a file version without reading it: path, size, times, inode.
fn identity_tag(path: &Path) -> Result<String> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let m = std::fs::metadata(path).map_err(io_err(path))?;
    let mut h = DefaultHasher::new();
    (path, m.len(), m.modified().ok()).hash(&mut h);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()).hash(&mut h);
    }
    Ok(format!("{:016x}", h.finish()))
}

/// Write `dst` atomically (temp file + rename). Losing a race to another
/// loader is fine: the winner wrote the same bytes.
fn write_copy(artifact: &Path, dir: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    let n = LOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".copy-{}-{n}.tmp", std::process::id()));
    std::fs::copy(artifact, &tmp).map_err(io_err(&tmp))?;
    if let Err(e) = std::fs::rename(&tmp, dst) {
        let _ = std::fs::remove_file(&tmp);
        if !dst.is_file() {
            return Err(io_err(dst)(e));
        }
    }
    Ok(())
}

/// Copy `artifact` to a unique path inside `shadow_dir`, deleted on drop: the
/// last resort when [`MAX_COPIES`] copies of one file are all mapped.
/// The extension follows the format, not the source name (`libx.so.1` has
/// extension `1`, and Windows appends `.dll` to names it does not recognise).
fn shadow_copy(id: &str, artifact: &Path, format: &Format, shadow_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(shadow_dir).map_err(io_err(shadow_dir))?;
    let n = LOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ext = format::shared_ext(format);
    let dst = shadow_dir.join(format!("{id}-{}-{n}.{ext}", std::process::id()));
    std::fs::copy(artifact, &dst).map_err(io_err(&dst))?;
    Ok(dst)
}

fn decl_str(what: &str, s: PwStr) -> Result<String> {
    if s.len > MAX_DECL_STR {
        return Err(PluginError::Load(format!("{what} longer than {MAX_DECL_STR} bytes")));
    }
    let bytes = unsafe { s.as_bytes() };
    String::from_utf8(bytes.to_vec()).map_err(|_| PluginError::Load(format!("{what} is not UTF-8")))
}

impl NativePlugin {
    /// Load `artifact`, validate its declaration table and create an instance.
    ///
    /// `placement` picks the file that is mapped (see [`Placement`]). The checks
    /// are the ones that keep the host sound: the file format (magic bytes), the
    /// ABI version and table size, the entry points, the SDK version.
    ///
    /// `config` is passed verbatim to the plugin's `new` (JSON by convention).
    pub fn load(id: &str, artifact: &Path, placement: Placement, config: &[u8], sink: LogSink) -> Result<Self> {
        let started = Instant::now();
        // Reject static libraries, Wasm and foreign images with a precise reason
        // instead of whatever `dlopen` makes of them.
        let format = format::check_loadable(artifact)?;
        // Choosing the image and mapping it is one step: two threads loading the
        // same file must not both see it unmapped and end up sharing it.
        static MAPPING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let mapping = MAPPING.lock().unwrap_or_else(|p| p.into_inner());
        let (path, shadow) = image(id, artifact, &format, placement)?;
        let not_a_plugin = |reason: String| PluginError::NotAPlugin { path: artifact.to_owned(), reason };

        // SAFETY: loading runs the library's initialisers. This is the trust
        // boundary of in-process loading; use `Isolation::Process` for code you
        // do not trust.
        let lib = unsafe { Library::new(&path) };
        drop(mapping);
        let lib = match lib {
            Ok(lib) => lib,
            Err(e) => {
                if let Some(s) = &shadow {
                    let _ = std::fs::remove_file(s);
                }
                return Err(PluginError::Load(e.to_string()));
            }
        };
        let mut this = NativePlugin {
            id: id.to_owned(),
            instance: std::ptr::null_mut(),
            decl: std::ptr::null(),
            info: PluginInfo {
                name: String::new(),
                version: semver::Version::new(0, 0, 0),
                sdk_version: semver::Version::new(0, 0, 0),
                backend: Backend::Native,
                engine: None,
                sandbox: None,
            },
            lib: Some(lib),
            path,
            shadow,
        };

        let lib = this.lib.as_ref().expect("just set");
        let entry: EntryFn = unsafe { lib.get::<EntryFn>(NATIVE_ENTRY) }
            .map(|s| *s)
            .map_err(|e| not_a_plugin(format!("missing `{NATIVE_ENTRY}`: {e}")))?;
        let decl = catch_unwind(|| unsafe { entry() }).map_err(|_| not_a_plugin("entry point panicked".into()))?;
        if decl.is_null() {
            return Err(not_a_plugin("entry point returned null".into()));
        }
        // Read only the first field before trusting the rest of the layout.
        let found = unsafe { std::ptr::read(std::ptr::addr_of!((*decl).abi_version)) };
        if found != ABI_VERSION {
            return Err(PluginError::AbiMismatch { expected: ABI_VERSION, found });
        }
        let d = unsafe { &*decl };
        if (d.struct_size as usize) < std::mem::size_of::<PwPluginDecl>() {
            return Err(not_a_plugin(format!("declaration table too small ({} bytes)", d.struct_size)));
        }
        let (Some(create), Some(_), Some(_), Some(_)) = (d.create, d.call, d.destroy, d.free_buf) else {
            return Err(not_a_plugin("declaration table has empty slots".into()));
        };
        this.decl = decl;
        this.info.name = decl_str("name", d.name)?;
        this.info.version = parse_version("plugin version", &decl_str("version", d.version)?)?;
        this.info.sdk_version = parse_version("SDK version", &decl_str("sdk_version", d.sdk_version)?)?;
        check_sdk(id, &this.info.sdk_version)?;

        let ctx: &'static HostCtx = Box::leak(Box::new(HostCtx { sink }));
        let host = PwHostApi { ctx: (ctx as *const HostCtx).cast_mut().cast(), log: Some(host_log) };
        let mut instance = std::ptr::null_mut();
        let mut err = PwBuf::EMPTY;
        let status = unsafe { create(&host, PwStr::from_bytes(config), &mut instance, &mut err) };
        let msg = this.take_buf(err)?;
        match status {
            STATUS_OK if !instance.is_null() => this.instance = instance,
            STATUS_OK => return Err(PluginError::Load("create returned a null instance".into())),
            other => return Err(this.status_error("create", other, msg)),
        }
        tracing::info!(
            plugin = id, name = %this.info.name, version = %this.info.version, sdk = %this.info.sdk_version,
            abi = ABI_VERSION, elapsed_ms = started.elapsed().as_millis() as u64, "native plugin loaded"
        );
        Ok(this)
    }

    /// Destroy the instance, close the library and check that it is gone.
    /// Idempotent: returns `None` once the library has been closed.
    fn shutdown(&mut self) -> Option<Release> {
        if !self.instance.is_null()
            && let Some(destroy) = self.decl().destroy
        {
            unsafe { destroy(std::mem::replace(&mut self.instance, std::ptr::null_mut())) };
        }
        let lib = self.lib.take()?;
        if let Err(e) = lib.close() {
            tracing::warn!(plugin = %self.id, error = %e, "dlclose failed");
        }
        let release = if still_mapped(&self.path) { Release::StillMapped } else { Release::Unmapped };
        // Best effort: on Windows the file stays locked while the DLL is mapped.
        if let Some(s) = &self.shadow {
            let _ = std::fs::remove_file(s);
        }
        if release == Release::StillMapped {
            tracing::warn!(plugin = %self.id, "native plugin unloaded, but its library is still mapped");
        } else {
            tracing::info!(plugin = %self.id, "native plugin unloaded and unmapped");
        }
        Some(release)
    }

    fn decl(&self) -> &PwPluginDecl {
        unsafe { &*self.decl }
    }

    /// Copy a plugin-owned buffer and give it back to the plugin's allocator.
    fn take_buf(&self, buf: PwBuf) -> Result<Vec<u8>> {
        if buf.ptr.is_null() {
            return Ok(Vec::new());
        }
        if buf.len > buf.cap || buf.len > MAX_OUTPUT {
            // Do not touch a buffer with a nonsensical shape; leak it.
            return Err(PluginError::Load(format!(
                "plugin returned an invalid buffer (len {}, cap {})",
                buf.len, buf.cap
            )));
        }
        let out = unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec();
        if let Some(free) = self.decl().free_buf {
            unsafe { free(buf) };
        }
        Ok(out)
    }

    fn status_error(&self, method: &str, status: PwStatus, msg: Vec<u8>) -> PluginError {
        let message = String::from_utf8_lossy(&msg).into_owned();
        match status {
            STATUS_ERROR => PluginError::Call { method: method.to_owned(), message },
            STATUS_PANIC => PluginError::Panicked(message),
            other => PluginError::Load(format!("plugin returned unknown status {other} from `{method}`")),
        }
    }
}

impl Plugin for NativePlugin {
    fn info(&self) -> &PluginInfo {
        &self.info
    }

    fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        let call = self.decl().call.expect("validated at load");
        let mut out = PwBuf::EMPTY;
        let status =
            unsafe { call(self.instance, PwStr::from_bytes(method.as_bytes()), PwStr::from_bytes(input), &mut out) };
        let bytes = self.take_buf(out)?;
        if status == STATUS_OK { Ok(bytes) } else { Err(self.status_error(method, status, bytes)) }
    }

    fn close(mut self: Box<Self>) -> Release {
        self.shutdown().unwrap_or(Release::Unmapped)
    }
}

impl Drop for NativePlugin {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Whether the image at `path` is still loaded, without loading it or
/// touching its reference count.
#[cfg(unix)]
fn still_mapped(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: `RTLD_NOLOAD` only looks the image up (no initialisers run); a
    // hit takes a reference, which the `dlclose` below gives back.
    let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_LAZY | libc::RTLD_NOLOAD) };
    if handle.is_null() {
        return false;
    }
    unsafe { libc::dlclose(handle) };
    true
}

#[cfg(windows)]
fn still_mapped(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleHandleExW};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut module = std::ptr::null_mut();
    // SAFETY: NUL-terminated path; `UNCHANGED_REFCOUNT` takes no reference.
    unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, wide.as_ptr(), &mut module) != 0 }
}

#[cfg(not(any(unix, windows)))]
fn still_mapped(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ptr::NonNull;
    use std::sync::{Arc, Mutex};

    use plugwright_abi::LOG_WARN;

    use super::*;

    thread_local! {
        static FREED: Cell<usize> = const { Cell::new(0) };
        static DESTROYED: Cell<usize> = const { Cell::new(0) };
    }

    /// Hand `bytes` over as a plugin-owned buffer; `fake_free` takes it back.
    fn into_buf(bytes: &[u8]) -> PwBuf {
        let mut v = std::mem::ManuallyDrop::new(bytes.to_vec());
        PwBuf { ptr: v.as_mut_ptr(), len: v.len(), cap: v.capacity() }
    }

    unsafe extern "C" fn fake_free(buf: PwBuf) {
        drop(unsafe { Vec::from_raw_parts(buf.ptr, buf.len, buf.cap) });
        FREED.with(|n| n.set(n.get() + 1));
    }

    unsafe extern "C" fn fake_destroy(_instance: *mut c_void) {
        DESTROYED.with(|n| n.set(n.get() + 1));
    }

    /// Stands in for a loaded plugin; the method name picks the reply.
    unsafe extern "C" fn fake_call(_instance: *mut c_void, method: PwStr, input: PwStr, out: *mut PwBuf) -> PwStatus {
        let dangling = NonNull::dangling().as_ptr();
        let (status, buf) = match unsafe { method.as_bytes() } {
            b"echo" => (STATUS_OK, into_buf(unsafe { input.as_bytes() })),
            b"nothing" => (STATUS_OK, PwBuf::EMPTY),
            b"fail" => (STATUS_ERROR, into_buf(b"bad input")),
            b"latin1" => (STATUS_ERROR, into_buf(b"caf\xe9")),
            b"panic" => (STATUS_PANIC, into_buf(b"boom")),
            b"weird" => (42, PwBuf::EMPTY),
            // Shapes the host must refuse without reading or freeing them.
            b"len-over-cap" => (STATUS_OK, PwBuf { ptr: dangling, len: 8, cap: 4 }),
            b"oversized" => (STATUS_OK, PwBuf { ptr: dangling, len: MAX_OUTPUT + 1, cap: MAX_OUTPUT + 1 }),
            _ => (STATUS_ERROR, into_buf(b"no such method")),
        };
        unsafe { out.write(buf) };
        status
    }

    static DECL: PwPluginDecl = PwPluginDecl {
        abi_version: ABI_VERSION,
        struct_size: std::mem::size_of::<PwPluginDecl>() as u32,
        sdk_version: PwStr::from_static("0.1.0"),
        name: PwStr::from_static("fake"),
        version: PwStr::from_static("1.0.0"),
        create: None,
        call: Some(fake_call),
        destroy: Some(fake_destroy),
        free_buf: Some(fake_free),
    };

    /// A plugin over `DECL` with no library behind it, so shutting it down
    /// never reaches `dlclose`.
    fn fake(instance: *mut c_void) -> NativePlugin {
        NativePlugin {
            id: "fake".into(),
            instance,
            decl: &DECL,
            info: PluginInfo {
                name: "fake".into(),
                version: semver::Version::new(1, 0, 0),
                sdk_version: semver::Version::new(0, 1, 0),
                backend: Backend::Native,
                engine: None,
                sandbox: None,
            },
            lib: None,
            path: PathBuf::from("fake"),
            shadow: None,
        }
    }

    /// Call `method` on a fresh fake; also returns how many buffers were freed.
    fn call(method: &str, input: &[u8]) -> (Result<Vec<u8>>, usize) {
        FREED.with(|n| n.set(0));
        let out = fake(std::ptr::null_mut()).call(method, input);
        (out, FREED.with(Cell::get))
    }

    fn sink() -> LogSink {
        Arc::new(|_: u32, _: &str| {})
    }

    fn load_err(artifact: &Path, placement: Placement) -> PluginError {
        match NativePlugin::load("demo", artifact, placement, b"{}", sink()) {
            Ok(_) => panic!("{} loaded", artifact.display()),
            Err(e) => e,
        }
    }

    #[test]
    fn call_returns_the_output_and_frees_it_with_the_plugin_allocator() {
        let (out, freed) = call("echo", b"payload");
        assert_eq!(out.unwrap(), b"payload");
        assert_eq!(freed, 1);
    }

    #[test]
    fn call_without_an_output_buffer_returns_empty() {
        let (out, freed) = call("nothing", b"x");
        assert!(out.unwrap().is_empty());
        assert_eq!(freed, 0);
    }

    #[test]
    fn error_status_becomes_a_call_error_carrying_the_message() {
        let (out, freed) = call("fail", b"");
        match out {
            Err(PluginError::Call { method, message }) => {
                assert_eq!(method, "fail");
                assert_eq!(message, "bad input");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(freed, 1);
    }

    #[test]
    fn non_utf8_error_message_is_decoded_lossily() {
        let (out, _) = call("latin1", b"");
        assert!(matches!(out, Err(PluginError::Call { message, .. }) if message == "caf\u{FFFD}"));
    }

    #[test]
    fn panic_status_becomes_panicked() {
        let (out, freed) = call("panic", b"");
        assert!(matches!(out, Err(PluginError::Panicked(m)) if m == "boom"));
        assert_eq!(freed, 1);
    }

    #[test]
    fn unknown_status_is_refused_naming_the_status_and_method() {
        match call("weird", b"").0 {
            Err(PluginError::Load(msg)) => assert_eq!(msg, "plugin returned unknown status 42 from `weird`"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn malformed_output_buffers_are_refused_without_being_touched() {
        for (method, shape) in [("len-over-cap", "len 8, cap 4"), ("oversized", "len 268435457")] {
            let (out, freed) = call(method, b"");
            match out {
                Err(PluginError::Load(msg)) => assert!(msg.contains("invalid buffer") && msg.contains(shape), "{msg}"),
                other => panic!("{method}: {other:?}"),
            }
            assert_eq!(freed, 0, "{method}");
        }
    }

    #[test]
    fn close_destroys_the_instance_exactly_once() {
        DESTROYED.with(|n| n.set(0));
        let plugin = Box::new(fake(NonNull::<u8>::dangling().as_ptr().cast()));
        // `close` runs the shutdown and `Drop` runs it again; only the first destroys.
        assert_eq!(plugin.close(), Release::Unmapped);
        assert_eq!(DESTROYED.with(Cell::get), 1);
    }

    #[test]
    fn drop_destroys_a_live_instance_and_skips_a_null_one() {
        DESTROYED.with(|n| n.set(0));
        drop(fake(std::ptr::null_mut()));
        assert_eq!(DESTROYED.with(Cell::get), 0);
        drop(fake(NonNull::<u8>::dangling().as_ptr().cast()));
        assert_eq!(DESTROYED.with(Cell::get), 1);
    }

    #[test]
    fn decl_str_accepts_utf8_up_to_the_limit() {
        assert_eq!(decl_str("name", PwStr::from_static("hello")).unwrap(), "hello");
        assert_eq!(decl_str("name", PwStr::EMPTY).unwrap(), "");
        let max = "x".repeat(MAX_DECL_STR);
        assert_eq!(decl_str("name", PwStr::from_bytes(max.as_bytes())).unwrap(), max);
    }

    #[test]
    fn decl_str_checks_the_length_before_reading() {
        // Reading this would be undefined behaviour; the bound check comes first.
        let bogus = PwStr { ptr: NonNull::dangling().as_ptr(), len: usize::MAX };
        match decl_str("version", bogus) {
            Err(PluginError::Load(msg)) => assert_eq!(msg, format!("version longer than {MAX_DECL_STR} bytes")),
            other => panic!("{other:?}"),
        }
        let long = "x".repeat(MAX_DECL_STR + 1);
        assert!(decl_str("name", PwStr::from_bytes(long.as_bytes())).is_err());
    }

    #[test]
    fn decl_str_rejects_invalid_utf8() {
        match decl_str("sdk_version", PwStr::from_bytes(b"0.1.\xff")) {
            Err(PluginError::Load(msg)) => assert_eq!(msg, "sdk_version is not UTF-8"),
            other => panic!("{other:?}"),
        }
    }

    type Lines = Arc<Mutex<Vec<(u32, String)>>>;

    fn recording_ctx() -> (HostCtx, Lines) {
        let lines = Lines::default();
        let seen = lines.clone();
        let ctx =
            HostCtx { sink: Arc::new(move |level: u32, msg: &str| seen.lock().unwrap().push((level, msg.to_owned()))) };
        (ctx, lines)
    }

    fn log(ctx: &HostCtx, level: u32, msg: &[u8]) {
        unsafe { host_log((ctx as *const HostCtx).cast_mut().cast(), level, PwStr::from_bytes(msg)) };
    }

    #[test]
    fn host_log_forwards_level_and_message() {
        let (ctx, lines) = recording_ctx();
        log(&ctx, LOG_WARN, b"disk almost full");
        log(&ctx, LOG_WARN + 1, b"");
        assert_eq!(*lines.lock().unwrap(), [(LOG_WARN, "disk almost full".to_owned()), (LOG_WARN + 1, String::new())]);
    }

    #[test]
    fn host_log_truncates_long_lines_and_replaces_invalid_utf8() {
        let (ctx, lines) = recording_ctx();
        // The cut lands inside a two-byte character.
        let mut long = vec![b'a'; MAX_LOG_LINE - 1];
        long.extend_from_slice("é and more".as_bytes());
        log(&ctx, LOG_WARN, &long);
        log(&ctx, LOG_WARN, b"caf\xe9");
        let lines = lines.lock().unwrap();
        assert_eq!(lines[0].1, format!("{}\u{FFFD}", "a".repeat(MAX_LOG_LINE - 1)));
        assert_eq!(lines[1].1, "caf\u{FFFD}");
    }

    #[test]
    fn host_log_ignores_a_null_context() {
        unsafe { host_log(std::ptr::null_mut(), LOG_WARN, PwStr::from_static("dropped")) };
    }

    #[test]
    fn host_log_contains_a_panicking_sink() {
        let ctx = HostCtx { sink: Arc::new(|_: u32, _: &str| panic!("subscriber bug")) };
        log(&ctx, LOG_WARN, b"must not unwind into the plugin");
    }

    #[test]
    fn shadow_copy_takes_its_extension_from_the_format() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("libdemo.so.1");
        std::fs::write(&artifact, b"image").unwrap();
        let shadow_dir = dir.path().join("nested").join("shadow");
        let cases = [
            (Format::Elf { kind: format::ElfKind::Shared, arch: None }, "so"),
            (Format::MachOFat { arches: vec![] }, "dylib"),
            (Format::Pe { dll: true, arch: None }, "dll"),
        ];
        for (format, ext) in cases {
            let copy = shadow_copy("demo", &artifact, &format, &shadow_dir).unwrap();
            assert_eq!(copy.parent(), Some(shadow_dir.as_path()));
            assert_eq!(copy.extension().and_then(|e| e.to_str()), Some(ext));
            assert!(copy.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("demo-")), "{copy:?}");
            assert_eq!(std::fs::read(&copy).unwrap(), b"image");
        }
    }

    #[test]
    fn shadow_copies_of_one_artifact_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("libdemo.dylib");
        std::fs::write(&artifact, b"image").unwrap();
        let format = Format::MachO { kind: format::MachKind::Dylib, arch: None };
        let a = shadow_copy("demo", &artifact, &format, dir.path()).unwrap();
        let b = shadow_copy("demo", &artifact, &format, dir.path()).unwrap();
        assert_ne!(a, b);
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn shadow_copy_of_a_missing_artifact_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let format = Format::Elf { kind: format::ElfKind::Shared, arch: None };
        let err = shadow_copy("demo", &dir.path().join("missing.so"), &format, dir.path()).unwrap_err();
        assert!(matches!(err, PluginError::Io { .. }), "{err:?}");
    }

    #[test]
    fn load_refuses_what_is_not_a_host_library_before_copying_it() {
        let dir = tempfile::tempdir().unwrap();
        let shadow_dir = dir.path().join("shadow");
        // Named like a library of this host; only the bytes give it away.
        let artifact = dir.path().join(format!("libdemo.{}", std::env::consts::DLL_EXTENSION));
        let mut foreign_elf = vec![0u8; 64];
        foreign_elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        foreign_elf[16] = 3; // ET_DYN
        foreign_elf[18] = if std::env::consts::ARCH == "x86_64" { 0xB7 } else { 0x3E };
        let cases: [(&[u8], &str); 5] = [
            (b"#!/bin/sh\n", "not a shared library"),
            (b"", "not a shared library"),
            (b"!<arch>\n", "static library"),
            (b"\0asm\x01\0\0\0", "kind = \"wasm\""),
            (&foreign_elf, "cannot be loaded on"),
        ];
        for (bytes, reason) in cases {
            std::fs::write(&artifact, bytes).unwrap();
            match load_err(&artifact, Placement::Copy { dir: &shadow_dir, tag: "abc" }) {
                PluginError::NotAPlugin { path, reason: r } => {
                    assert_eq!(path, artifact);
                    assert!(r.contains(reason), "{r:?} lacks {reason:?}");
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(!shadow_dir.exists(), "a refused artifact was copied");
    }

    #[test]
    fn load_reports_a_missing_artifact_as_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("missing.so");
        match load_err(&artifact, Placement::InPlace { copies: None }) {
            PluginError::Io { path, .. } => assert_eq!(path, artifact),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn in_place_maps_the_file_itself_without_copying() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("libdemo.so");
        std::fs::write(&artifact, b"image").unwrap();
        let elf = Format::Elf { kind: format::ElfKind::Shared, arch: None };
        let (path, private) = image("demo", &artifact, &elf, Placement::InPlace { copies: None }).unwrap();
        assert_eq!((path, private), (std::fs::canonicalize(&artifact).unwrap(), None));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "nothing was copied");
    }

    #[test]
    fn copies_are_named_by_tag_written_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("libdemo.so.1");
        std::fs::write(&artifact, b"v1").unwrap();
        let loaded = dir.path().join("loaded");
        let elf = Format::Elf { kind: format::ElfKind::Shared, arch: None };
        let copy = Placement::Copy { dir: &loaded, tag: "ab12/../cd" };
        let (path, private) = image("demo", &artifact, &elf, copy).unwrap();
        assert_eq!((path.clone(), private), (loaded.join("ab12cd.so"), None), "the tag is sanitised");
        assert_eq!(std::fs::read(&path).unwrap(), b"v1");

        // Same tag, same file: not rewritten (the tag stands for the content).
        std::fs::write(&artifact, b"v2").unwrap();
        assert_eq!(image("demo", &artifact, &elf, copy).unwrap().0, path);
        assert_eq!(std::fs::read(&path).unwrap(), b"v1");
        let names: Vec<_> = std::fs::read_dir(&loaded).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["ab12cd.so"], "no temp files left behind");

        let bad = Placement::Copy { dir: &loaded, tag: "../" };
        assert!(matches!(image("demo", &artifact, &elf, bad), Err(PluginError::Load(_))));
    }

    #[test]
    fn unused_copies_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["a.so", "b.dylib", ".c.tmp"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert_eq!(prune_copies(dir.path()), 3);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "directories are left alone");
        assert_eq!(prune_copies(&dir.path().join("missing")), 0);
    }

    #[test]
    fn a_file_that_was_never_loaded_is_not_mapped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("never.{}", std::env::consts::DLL_EXTENSION));
        assert!(!still_mapped(&path));
        std::fs::write(&path, b"not a library").unwrap();
        assert!(!still_mapped(&path));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_with_an_interior_nul_is_not_mapped() {
        assert!(!still_mapped(Path::new("lib\0demo.so")));
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn libsystem_is_reported_as_mapped() {
        // Every macOS process links libSystem, so the probe must find it.
        assert!(still_mapped(Path::new("/usr/lib/libSystem.B.dylib")));
    }
}
