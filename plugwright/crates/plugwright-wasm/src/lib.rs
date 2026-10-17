//! Engine-agnostic Wasm plugin layer.
//!
//! An engine adapter implements three small traits with primitive operations
//! ([`WasmEngine`] → [`WasmModule`] → [`WasmInstance`]). The plugwright ABI
//! (`pw_alloc`, `pw_call`, …), bounds checks, limits and trap recovery live in
//! [`WasmPlugin`] and are shared by every engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use plugwright_abi::wasm::{self as abi, unpack};
use plugwright_abi::{ABI_VERSION, STATUS_ERROR, STATUS_OK};

pub use plugwright_core::WasmError;
use plugwright_core::error::{PluginError, Result};
use plugwright_core::logging::LogSink;
use plugwright_core::plugin::{Backend, Plugin, PluginInfo, check_sdk, parse_version};

/// Values crossing the core-Wasm ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmVal {
    I32(i32),
    I64(i64),
}

/// Resource limits applied to every instance.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WasmLimits {
    /// Fuel granted at the start of every call (≈ instructions). `None` = unlimited.
    pub fuel: Option<u64>,
    /// Maximum linear memory in bytes.
    pub max_memory_bytes: Option<usize>,
    /// Wall-clock limit per call (engines without interruption rely on fuel).
    #[serde(with = "opt_millis", rename = "timeout_ms")]
    pub timeout: Option<Duration>,
    /// Maximum Wasm stack in bytes (engines that support it).
    pub max_stack_bytes: Option<usize>,
}

impl Default for WasmLimits {
    fn default() -> Self {
        WasmLimits {
            fuel: Some(1_000_000_000),
            max_memory_bytes: Some(64 << 20),
            timeout: Some(Duration::from_secs(5)),
            max_stack_bytes: Some(512 << 10),
        }
    }
}

mod opt_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(d) => s.serialize_u64(d.as_millis() as u64),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Ok(Option::<u64>::deserialize(d)?.map(Duration::from_millis))
    }
}

/// A Wasm engine. Implement this to plug in another runtime.
pub trait WasmEngine: Send + Sync {
    fn name(&self) -> &str;
    fn compile(&self, wasm: &[u8]) -> Result<Arc<dyn WasmModule>, WasmError>;

    /// [`compile`](Self::compile), reusing what an earlier compile of the
    /// same bytes left in `dir`, for engines that can keep compiled code
    /// across processes. `key` is the sha256 of `wasm` (lowercase hex); `dir`
    /// exists and belongs to this engine. Whatever is stored must be checked
    /// against this engine's configuration when read back.
    fn compile_cached(&self, wasm: &[u8], key: &str, dir: &Path) -> Result<Arc<dyn WasmModule>, WasmError> {
        let _ = (key, dir);
        self.compile(wasm)
    }
}

/// Compiled modules by engine and content. Plugins loaded from the same bytes
/// on the same engine share one compiled module while any of them is alive;
/// engines that can ([`WasmEngine::compile_cached`]) also keep compiled code
/// in `dir/<engine name>/` for the next process.
#[derive(Default)]
pub struct ModuleCache {
    dir: Option<PathBuf>,
    live: Mutex<HashMap<(usize, String), LiveModule>>,
}

/// A compiled module and the engine it belongs to, both weakly held.
type LiveModule = (Weak<dyn WasmEngine>, Weak<dyn WasmModule>);

impl ModuleCache {
    /// `dir: None` keeps compiled modules in memory only.
    pub fn new(dir: Option<PathBuf>) -> Self {
        ModuleCache { dir, live: Mutex::default() }
    }

    pub fn compile(&self, engine: &Arc<dyn WasmEngine>, wasm: &[u8]) -> Result<Arc<dyn WasmModule>, WasmError> {
        use sha2::Digest;
        let key: String = sha2::Sha256::digest(wasm).iter().map(|b| format!("{b:02x}")).collect();
        // Engines are told apart by address; the stored `Weak` keeps that
        // address from being reused by another engine while the entry exists.
        let slot = (Arc::as_ptr(engine) as *const () as usize, key);
        let live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((e, m)) = live.get(&slot)
            && e.strong_count() > 0
            && let Some(m) = m.upgrade()
        {
            return Ok(m);
        }
        drop(live);
        let module = match &self.dir {
            Some(dir) => {
                let dir = dir.join(engine_dir(engine.name()));
                match std::fs::create_dir_all(&dir) {
                    Ok(()) => engine.compile_cached(wasm, &slot.1, &dir)?,
                    Err(e) => {
                        tracing::warn!(dir = %dir.display(), error = %e, "cannot keep compiled wasm");
                        engine.compile(wasm)?
                    }
                }
            }
            None => engine.compile(wasm)?,
        };
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        live.retain(|_, (e, m)| e.strong_count() > 0 && m.strong_count() > 0);
        live.insert(slot, (Arc::downgrade(engine), Arc::downgrade(&module)));
        Ok(module)
    }

    /// Remove compiled code kept on disk for modules whose sha256 is not in
    /// `keep`. Returns the number of files removed.
    pub fn prune(&self, keep: &std::collections::HashSet<String>) -> usize {
        let Some(dir) = &self.dir else { return 0 };
        let engines = std::fs::read_dir(dir).into_iter().flatten().flatten();
        let files = engines.flat_map(|e| std::fs::read_dir(e.path()).into_iter().flatten().flatten());
        files
            .filter(|f| {
                let name = f.file_name().to_string_lossy().into_owned();
                !name.get(..64).is_some_and(|sha| keep.contains(sha))
            })
            .filter(|f| std::fs::remove_file(f.path()).is_ok())
            .count()
    }
}

/// Files of compiled code in a [`ModuleCache`] directory, for engines that
/// implement [`WasmEngine::compile_cached`]: written atomically, prefixed with
/// their sha256 and checked against it when read, so a torn or damaged file is
/// a cache miss instead of code handed to the engine.
pub mod stored {
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use sha2::{Digest, Sha256};

    /// Write `code` to `path` (temp file + rename).
    pub fn write(path: &Path, code: &[u8]) -> std::io::Result<()> {
        static N: AtomicU64 = AtomicU64::new(0);
        let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let mut data = Sha256::digest(code).to_vec();
        data.extend_from_slice(code);
        std::fs::write(&tmp, data).and_then(|()| std::fs::rename(&tmp, path)).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// The code [`write`] stored at `path`; `None` when the file is missing,
    /// short or does not match its hash.
    pub fn read(path: &Path) -> Option<Vec<u8>> {
        let mut data = std::fs::read(path).ok()?;
        if data.len() < 32 || Sha256::digest(&data[32..])[..] != data[..32] {
            return None;
        }
        data.drain(..32);
        Some(data)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn stored_code_round_trips_and_damage_is_a_miss() {
            let dir = tempfile::tempdir().unwrap();
            let f = dir.path().join("m.bin");
            assert_eq!(super::read(&f), None);
            super::write(&f, b"machine code").unwrap();
            assert_eq!(super::read(&f).as_deref(), Some(&b"machine code"[..]));
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp files left");
            let mut raw = std::fs::read(&f).unwrap();
            *raw.last_mut().unwrap() ^= 1;
            std::fs::write(&f, &raw).unwrap();
            assert_eq!(super::read(&f), None, "flipped bit");
            std::fs::write(&f, &raw[..31]).unwrap();
            assert_eq!(super::read(&f), None, "truncated");
            super::write(&f, b"").unwrap();
            assert_eq!(super::read(&f).as_deref(), Some(&b""[..]));
        }
    }
}

/// An engine name as a directory name.
fn engine_dir(name: &str) -> String {
    let n: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(32).collect();
    if n.is_empty() { "engine".to_owned() } else { n }
}

/// A compiled module that can be instantiated many times.
pub trait WasmModule: Send + Sync {
    /// Instantiate with the single host import `plugwright.log` wired to `log`.
    /// Any other import must make instantiation fail (deny by default).
    fn instantiate(&self, limits: &WasmLimits, log: LogSink) -> Result<Box<dyn WasmInstance>, WasmError>;
}

/// A live instance. All methods are primitive; the ABI lives in [`WasmPlugin`].
pub trait WasmInstance: Send {
    /// Reset per-call budgets (fuel, deadline). Called before every guest call sequence.
    fn begin_call(&mut self) -> Result<(), WasmError>;
    /// Call an exported function. Returns its single result, if any.
    fn call(&mut self, func: &str, args: &[WasmVal]) -> Result<Option<WasmVal>, WasmError>;
    /// Bounds-checked read from exported `memory`.
    fn read(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>, WasmError>;
    /// Bounds-checked write to exported `memory`.
    fn write(&mut self, ptr: u32, data: &[u8]) -> Result<(), WasmError>;
}

/// Read a guest log line from memory with a length cap; shared by adapters.
pub const MAX_GUEST_LOG: u32 = 64 << 10;

/// A Wasm plugin running on any [`WasmEngine`]. Implements [`Plugin`].
pub struct WasmPlugin {
    id: String,
    module: Arc<dyn WasmModule>,
    limits: WasmLimits,
    log: LogSink,
    config: Vec<u8>,
    instance: Option<Box<dyn WasmInstance>>,
    info: PluginInfo,
}

impl WasmPlugin {
    pub fn load(
        id: &str,
        engine: &dyn WasmEngine,
        wasm: &[u8],
        limits: WasmLimits,
        config: &[u8],
        log: LogSink,
    ) -> Result<Self> {
        let started = Instant::now();
        let module = engine.compile(wasm)?;
        Self::from_module(id, engine.name(), module, limits, config, log, started)
    }

    /// Load from a module compiled earlier (see [`ModuleCache`]).
    pub fn with_module(
        id: &str,
        engine: &str,
        module: Arc<dyn WasmModule>,
        limits: WasmLimits,
        config: &[u8],
        log: LogSink,
    ) -> Result<Self> {
        Self::from_module(id, engine, module, limits, config, log, Instant::now())
    }

    fn from_module(
        id: &str,
        engine: &str,
        module: Arc<dyn WasmModule>,
        limits: WasmLimits,
        config: &[u8],
        log: LogSink,
        started: Instant,
    ) -> Result<Self> {
        let mut this = WasmPlugin {
            id: id.to_owned(),
            module,
            limits,
            log,
            config: config.to_vec(),
            instance: None,
            info: PluginInfo {
                name: String::new(),
                version: semver::Version::new(0, 0, 0),
                sdk_version: semver::Version::new(0, 0, 0),
                backend: Backend::Wasm,
                engine: Some(engine.to_owned()),
                sandbox: None,
            },
        };
        this.boot()?;
        tracing::info!(
            plugin = id, engine, name = %this.info.name, version = %this.info.version,
            sdk = %this.info.sdk_version, elapsed_ms = started.elapsed().as_millis() as u64, "wasm plugin loaded"
        );
        Ok(this)
    }

    /// Instantiate, verify ABI + metadata and run `pw_init`.
    fn boot(&mut self) -> Result<()> {
        let mut inst = self.module.instantiate(&self.limits, Arc::clone(&self.log))?;
        inst.begin_call()?;
        match inst.call(abi::EXPORT_ABI_VERSION, &[])? {
            Some(WasmVal::I32(v)) if v as u32 == ABI_VERSION => {}
            Some(WasmVal::I32(v)) => return Err(PluginError::AbiMismatch { expected: ABI_VERSION, found: v as u32 }),
            _ => return Err(WasmError::Abi(format!("`{}` must return i32", abi::EXPORT_ABI_VERSION)).into()),
        }
        let meta = take_buffer(inst.as_mut(), abi::EXPORT_META, &[], 1024)?;
        let meta = String::from_utf8(meta).map_err(|_| WasmError::Abi("metadata is not UTF-8".into()))?;
        let mut parts = meta.split('\0');
        let (Some(name), Some(version), Some(sdk), None) = (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(WasmError::Abi("metadata must be `name\\0version\\0sdk_version`".into()).into());
        };
        self.info.name = name.to_owned();
        self.info.version = parse_version("plugin version", version)?;
        self.info.sdk_version = parse_version("SDK version", sdk)?;
        check_sdk(&self.id, &self.info.sdk_version)?;

        let cfg = put(inst.as_mut(), &self.config)?;
        let out = take_buffer(inst.as_mut(), abi::EXPORT_INIT, &[cfg.0, cfg.1], self.max_output())?;
        let (status, payload) = split_status(out)?;
        if status != STATUS_OK {
            return Err(PluginError::Load(String::from_utf8_lossy(&payload).into_owned()));
        }
        self.instance = Some(inst);
        Ok(())
    }

    fn max_output(&self) -> u32 {
        self.limits.max_memory_bytes.map_or(u32::MAX, |m| m.min(u32::MAX as usize) as u32)
    }

    fn call_inner(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        if self.instance.is_none() {
            tracing::warn!(plugin = %self.id, "re-instantiating wasm plugin after a previous failure");
            self.boot()?;
        }
        let max = self.max_output();
        let inst = self.instance.as_mut().expect("booted").as_mut();
        inst.begin_call()?;
        let m = put(inst, method.as_bytes())?;
        let i = put(inst, input)?;
        let out = take_buffer(inst, abi::EXPORT_CALL, &[m.0, m.1, i.0, i.1], max)?;
        match split_status(out)? {
            (STATUS_OK, payload) => Ok(payload),
            (STATUS_ERROR, payload) => {
                Err(PluginError::Call { method: method.to_owned(), message: String::from_utf8_lossy(&payload).into() })
            }
            (other, _) => Err(WasmError::Abi(format!("unknown status {other}")).into()),
        }
    }
}

/// Allocate a guest buffer and copy `data` into it.
fn put(inst: &mut dyn WasmInstance, data: &[u8]) -> Result<(WasmVal, WasmVal), WasmError> {
    let len = i32::try_from(data.len()).map_err(|_| WasmError::Abi("input larger than 2 GiB".into()))?;
    let ptr = match inst.call(abi::EXPORT_ALLOC, &[WasmVal::I32(len)])? {
        Some(WasmVal::I32(p)) => p,
        _ => return Err(WasmError::Abi(format!("`{}` must return i32", abi::EXPORT_ALLOC))),
    };
    if !data.is_empty() {
        inst.write(ptr as u32, data)?;
    }
    Ok((WasmVal::I32(ptr), WasmVal::I32(len)))
}

/// Call `func`, which returns a packed `(ptr, len)`, copy the buffer out and free it.
fn take_buffer(inst: &mut dyn WasmInstance, func: &str, args: &[WasmVal], max: u32) -> Result<Vec<u8>, WasmError> {
    let packed = match inst.call(func, args)? {
        Some(WasmVal::I64(v)) => v,
        _ => return Err(WasmError::Abi(format!("`{func}` must return i64"))),
    };
    let (ptr, len) = unpack(packed);
    if len > max {
        return Err(WasmError::Abi(format!("`{func}` returned {len} bytes, limit is {max}")));
    }
    let bytes = inst.read(ptr, len)?;
    inst.call(abi::EXPORT_FREE, &[WasmVal::I32(ptr as i32), WasmVal::I32(len as i32)])?;
    Ok(bytes)
}

fn split_status(mut out: Vec<u8>) -> Result<(u32, Vec<u8>), WasmError> {
    if out.is_empty() {
        return Err(WasmError::Abi("empty result buffer".into()));
    }
    let status = out.remove(0) as u32;
    Ok((status, out))
}

impl Plugin for WasmPlugin {
    fn info(&self) -> &PluginInfo {
        &self.info
    }

    fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        let res = self.call_inner(method, input);
        if let Err(PluginError::Wasm(e)) = &res
            && e.poisons_instance()
        {
            tracing::warn!(plugin = %self.id, error = %e, "discarding wasm instance");
            self.instance = None;
        }
        res
    }
}

impl Drop for WasmPlugin {
    fn drop(&mut self) {
        tracing::info!(plugin = %self.id, "wasm plugin unloaded");
    }
}

/// Conversions between [`WasmVal`] and an engine's own value enum (every engine
/// names the variants `I32`/`I64`). Expands to `to_engine_vals` and
/// `from_engine_result` in the calling module; the orphan rule rules out `From`
/// impls in the adapter crates.
#[macro_export]
macro_rules! wasm_val_bridge {
    ($val:ident) => {
        fn to_engine_vals(args: &[$crate::WasmVal]) -> ::std::vec::Vec<$val> {
            args.iter()
                .map(|a| match *a {
                    $crate::WasmVal::I32(v) => $val::I32(v),
                    $crate::WasmVal::I64(v) => $val::I64(v),
                })
                .collect()
        }

        /// The first result of `func`, if any; other value types break the ABI.
        fn from_engine_result(
            func: &str,
            results: &[$val],
        ) -> ::std::result::Result<::std::option::Option<$crate::WasmVal>, $crate::WasmError> {
            match results.first() {
                Some($val::I32(v)) => Ok(Some($crate::WasmVal::I32(*v))),
                Some($val::I64(v)) => Ok(Some($crate::WasmVal::I64(*v))),
                Some(other) => Err($crate::WasmError::Abi(format!("`{func}` returned unsupported value {other:?}"))),
                None => Ok(None),
            }
        }
    };
}

#[cfg(test)]
mod module_cache {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// Counts compiles; with `disk`, records `compile_cached` calls and leaves a file.
    #[derive(Default)]
    struct Counting {
        compiles: AtomicU32,
        disk: bool,
        cached: Mutex<Vec<(String, PathBuf)>>,
    }

    struct Compiled;

    impl WasmModule for Compiled {
        fn instantiate(&self, _: &WasmLimits, _: LogSink) -> Result<Box<dyn WasmInstance>, WasmError> {
            Err(WasmError::Instantiate("not a real module".into()))
        }
    }

    impl WasmEngine for Counting {
        fn name(&self) -> &str {
            "count/../ing"
        }
        fn compile(&self, _: &[u8]) -> Result<Arc<dyn WasmModule>, WasmError> {
            self.compiles.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(Compiled))
        }
        fn compile_cached(&self, wasm: &[u8], key: &str, dir: &Path) -> Result<Arc<dyn WasmModule>, WasmError> {
            if !self.disk {
                return self.compile(wasm);
            }
            std::fs::write(dir.join(format!("{key}.bin")), b"code").unwrap();
            self.cached.lock().unwrap().push((key.to_owned(), dir.to_owned()));
            self.compile(wasm)
        }
    }

    fn compiles(c: &Counting) -> u32 {
        c.compiles.load(Ordering::Relaxed)
    }

    #[test]
    fn one_module_per_engine_and_content_while_it_is_alive() {
        let counting = Arc::new(Counting::default());
        let engine: Arc<dyn WasmEngine> = counting.clone();
        let cache = ModuleCache::new(None);
        let a = cache.compile(&engine, b"one").unwrap();
        let b = cache.compile(&engine, b"one").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(compiles(&counting), 1);
        let _c = cache.compile(&engine, b"two").unwrap();
        assert_eq!(compiles(&counting), 2, "other bytes, other module");

        let other = Arc::new(Counting::default());
        let other_engine: Arc<dyn WasmEngine> = other.clone();
        let d = cache.compile(&other_engine, b"one").unwrap();
        assert!(!Arc::ptr_eq(&a, &d), "a module never crosses engines, even of the same name");
        assert_eq!(compiles(&other), 1);

        drop((a, b));
        cache.compile(&engine, b"one").unwrap();
        assert_eq!(compiles(&counting), 3, "a module nobody holds is not kept");
    }

    #[test]
    fn compiled_code_on_disk_is_keyed_by_content_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let counting = Arc::new(Counting { disk: true, ..Counting::default() });
        let engine: Arc<dyn WasmEngine> = counting.clone();
        let cache = ModuleCache::new(Some(dir.path().join("compiled")));
        let _one = cache.compile(&engine, b"abc").unwrap();
        let _two = cache.compile(&engine, b"xyz").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let engine_dir = dir.path().join("compiled/counting"); // the name, sanitised
        let calls = counting.cached.lock().unwrap().clone();
        assert_eq!(calls[0], (abc.to_owned(), engine_dir.clone()), "{calls:?}");
        assert_eq!(std::fs::read_dir(&engine_dir).unwrap().count(), 2);
        assert_eq!(cache.prune(&[abc.to_owned()].into()), 1);
        assert!(engine_dir.join(format!("{abc}.bin")).is_file());
        assert_eq!(ModuleCache::new(None).prune(&Default::default()), 0);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use plugwright_abi::wasm::pack;
    use plugwright_core::host_sdk_version;

    use super::*;

    /// Linear memory of every mock guest instance.
    const MEM: usize = 64 << 10;

    /// What a scripted guest export does when the host calls it.
    #[derive(Clone, Debug)]
    enum Reply {
        /// Allocate a guest buffer holding these bytes and return it packed.
        Buf(Vec<u8>),
        /// Return this packed `(ptr, len)` verbatim.
        Packed(i64),
        /// Return this raw value.
        Ret(Option<WasmVal>),
        /// Fail the export call (trap, fuel, …).
        Fail(WasmError),
    }

    fn status(code: u32, payload: &[u8]) -> Reply {
        let mut buf = vec![code as u8];
        buf.extend_from_slice(payload);
        Reply::Buf(buf)
    }

    fn ok(payload: &[u8]) -> Reply {
        status(STATUS_OK, payload)
    }

    fn meta(name: &str, version: &str, sdk: &str) -> Vec<u8> {
        format!("{name}\0{version}\0{sdk}").into_bytes()
    }

    /// `pw_call` behaviour: `(method, input, nth call on this instance) -> reply`.
    type CallFn = Arc<dyn Fn(&str, &[u8], u32) -> Reply + Send + Sync>;

    fn on_call(f: impl Fn(&str, &[u8], u32) -> Reply + Send + Sync + 'static) -> CallFn {
        Arc::new(f)
    }

    /// Scripted behaviour of the mock engine and its guests; editable between calls.
    #[derive(Clone)]
    struct Script {
        abi: Reply,
        meta: Reply,
        init: Reply,
        call: CallFn,
        /// `None` = a bump allocator over the guest memory.
        alloc: Option<Reply>,
        missing: Vec<&'static str>,
        begin_call: Option<WasmError>,
        compile: Option<WasmError>,
        instantiate: Option<WasmError>,
    }

    impl Default for Script {
        fn default() -> Self {
            Script {
                abi: Reply::Ret(Some(WasmVal::I32(ABI_VERSION as i32))),
                meta: Reply::Buf(meta("demo", "1.2.3", &host_sdk_version().to_string())),
                init: ok(b""),
                call: on_call(|method, input, n| match method {
                    "echo" => ok(input),
                    "count" => ok(n.to_string().as_bytes()),
                    other => status(STATUS_ERROR, format!("no method {other}").as_bytes()),
                }),
                alloc: None,
                missing: Vec::new(),
                begin_call: None,
                compile: None,
                instantiate: None,
            }
        }
    }

    /// Everything the host did to the guests, shared across instances.
    #[derive(Debug, Default)]
    struct Trace {
        instances: u32,
        begin_calls: u32,
        /// Buffers handed to the host through a packed result, in order.
        returned: Vec<(u32, u32)>,
        freed: Vec<(i32, i32)>,
        allocs: Vec<i32>,
        writes: Vec<(u32, usize)>,
        configs: Vec<Vec<u8>>,
        calls: Vec<(String, Vec<u8>)>,
    }

    /// Engine and module at once; instances share its script and trace.
    #[derive(Clone, Default)]
    struct Mock {
        script: Arc<Mutex<Script>>,
        trace: Arc<Mutex<Trace>>,
    }

    impl Mock {
        fn new(script: Script) -> Self {
            Mock { script: Arc::new(Mutex::new(script)), trace: Arc::default() }
        }

        fn load(&self) -> Result<WasmPlugin> {
            self.load_with(WasmLimits::default(), b"cfg")
        }

        fn load_with(&self, limits: WasmLimits, config: &[u8]) -> Result<WasmPlugin> {
            WasmPlugin::load("demo-id", self, b"\0asm", limits, config, Arc::new(|_, _| {}))
        }

        fn script(&self) -> Script {
            self.script.lock().unwrap().clone()
        }

        fn edit(&self, f: impl FnOnce(&mut Script)) {
            f(&mut self.script.lock().unwrap());
        }

        fn trace(&self) -> MutexGuard<'_, Trace> {
            self.trace.lock().unwrap()
        }
    }

    impl WasmEngine for Mock {
        fn name(&self) -> &str {
            "mock"
        }

        fn compile(&self, wasm: &[u8]) -> Result<Arc<dyn WasmModule>, WasmError> {
            assert_eq!(wasm, b"\0asm");
            match self.script().compile {
                Some(e) => Err(e),
                None => Ok(Arc::new(self.clone())),
            }
        }
    }

    impl WasmModule for Mock {
        fn instantiate(&self, _limits: &WasmLimits, _log: LogSink) -> Result<Box<dyn WasmInstance>, WasmError> {
            if let Some(e) = self.script().instantiate {
                return Err(e);
            }
            self.trace().instances += 1;
            Ok(Box::new(Guest { mock: self.clone(), mem: vec![0; MEM], next: 8, calls: 0 }))
        }
    }

    /// An in-memory guest: bounds-checked linear memory, a bump allocator and a call counter.
    struct Guest {
        mock: Mock,
        mem: Vec<u8>,
        next: u32,
        calls: u32,
    }

    fn i32_args(func: &str, args: &[WasmVal]) -> Vec<u32> {
        args.iter()
            .map(|a| match *a {
                WasmVal::I32(v) => v as u32,
                WasmVal::I64(_) => panic!("`{func}` takes i32 arguments, got {args:?}"),
            })
            .collect()
    }

    impl Guest {
        fn range(&self, ptr: u32, len: u32) -> Result<std::ops::Range<usize>, WasmError> {
            let end = ptr.checked_add(len).ok_or(WasmError::OutOfBounds)?;
            if end as usize > self.mem.len() {
                return Err(WasmError::OutOfBounds);
            }
            Ok(ptr as usize..end as usize)
        }

        fn alloc(&mut self, len: u32) -> u32 {
            let ptr = self.next;
            self.next += len.max(1);
            assert!(self.next as usize <= MEM, "mock guest ran out of memory");
            ptr
        }

        fn reply(&mut self, reply: Reply) -> Result<Option<WasmVal>, WasmError> {
            match reply {
                Reply::Buf(bytes) => {
                    let len = bytes.len() as u32;
                    let ptr = self.alloc(len);
                    self.mem[ptr as usize..(ptr + len) as usize].copy_from_slice(&bytes);
                    self.mock.trace().returned.push((ptr, len));
                    Ok(Some(WasmVal::I64(pack(ptr, len))))
                }
                Reply::Packed(v) => {
                    self.mock.trace().returned.push(unpack(v));
                    Ok(Some(WasmVal::I64(v)))
                }
                Reply::Ret(v) => Ok(v),
                Reply::Fail(e) => Err(e),
            }
        }
    }

    impl WasmInstance for Guest {
        fn begin_call(&mut self) -> Result<(), WasmError> {
            self.mock.trace().begin_calls += 1;
            self.mock.script().begin_call.map_or(Ok(()), Err)
        }

        fn call(&mut self, func: &str, args: &[WasmVal]) -> Result<Option<WasmVal>, WasmError> {
            let script = self.mock.script();
            if script.missing.contains(&func) {
                return Err(WasmError::MissingExport(func.into()));
            }
            let a = i32_args(func, args);
            match func {
                abi::EXPORT_ABI_VERSION => self.reply(script.abi),
                abi::EXPORT_META => self.reply(script.meta),
                abi::EXPORT_ALLOC => {
                    self.mock.trace().allocs.push(a[0] as i32);
                    match script.alloc {
                        Some(r) => self.reply(r),
                        None => Ok(Some(WasmVal::I32(self.alloc(a[0]) as i32))),
                    }
                }
                abi::EXPORT_FREE => {
                    self.mock.trace().freed.push((a[0] as i32, a[1] as i32));
                    Ok(None)
                }
                abi::EXPORT_INIT => {
                    let config = self.read(a[0], a[1])?;
                    self.mock.trace().configs.push(config);
                    self.reply(script.init)
                }
                abi::EXPORT_CALL => {
                    let method = String::from_utf8(self.read(a[0], a[1])?).expect("host sends UTF-8 method names");
                    let input = self.read(a[2], a[3])?;
                    self.calls += 1;
                    let reply = (script.call)(&method, &input, self.calls);
                    self.mock.trace().calls.push((method, input));
                    self.reply(reply)
                }
                _ => Err(WasmError::MissingExport(func.into())),
            }
        }

        fn read(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>, WasmError> {
            let range = self.range(ptr, len)?;
            Ok(self.mem[range].to_vec())
        }

        fn write(&mut self, ptr: u32, data: &[u8]) -> Result<(), WasmError> {
            self.mock.trace().writes.push((ptr, data.len()));
            let len = u32::try_from(data.len()).map_err(|_| WasmError::OutOfBounds)?;
            let range = self.range(ptr, len)?;
            self.mem[range].copy_from_slice(data);
            Ok(())
        }
    }

    fn load_err(mock: &Mock) -> PluginError {
        match mock.load() {
            Ok(_) => panic!("load unexpectedly succeeded"),
            Err(e) => e,
        }
    }

    fn wasm_err(e: PluginError) -> WasmError {
        match e {
            PluginError::Wasm(w) => w,
            other => panic!("expected a wasm error, got {other:?}"),
        }
    }

    fn abi_err(msg: &str) -> WasmError {
        WasmError::Abi(msg.into())
    }

    fn with(f: impl FnOnce(&mut Script)) -> Mock {
        let mut script = Script::default();
        f(&mut script);
        Mock::new(script)
    }

    fn call_ok(p: &mut WasmPlugin, method: &str, input: &[u8]) -> Vec<u8> {
        p.call(method, input).unwrap_or_else(|e| panic!("`{method}` failed: {e}"))
    }

    // --- happy path -------------------------------------------------------

    #[test]
    fn load_parses_metadata_and_runs_init_with_config() {
        let mock = Mock::default();
        let p = mock.load().unwrap();
        let info = p.info();
        assert_eq!(info.name, "demo");
        assert_eq!(info.version, semver::Version::new(1, 2, 3));
        assert_eq!(info.sdk_version, host_sdk_version());
        assert_eq!(info.backend, Backend::Wasm);
        assert_eq!(info.engine.as_deref(), Some("mock"));
        assert_eq!(info.sandbox, None);
        let t = mock.trace();
        assert_eq!(t.instances, 1);
        assert_eq!(t.begin_calls, 1);
        assert_eq!(t.configs, [b"cfg".to_vec()]);
    }

    #[test]
    fn call_passes_method_and_input_and_returns_ok_payload() {
        let mock = Mock::default();
        let mut p = mock.load().unwrap();
        assert_eq!(call_ok(&mut p, "echo", b"hello"), b"hello");
        assert_eq!(call_ok(&mut p, "echo", b""), b"");
        let t = mock.trace();
        assert_eq!(t.calls, [("echo".to_owned(), b"hello".to_vec()), ("echo".to_owned(), Vec::new())]);
        assert_eq!(t.begin_calls, 3, "one budget reset for load and one per call");
    }

    #[test]
    fn every_returned_buffer_is_freed_with_its_ptr_and_len() {
        let mock = Mock::default();
        let mut p = mock.load().unwrap();
        call_ok(&mut p, "echo", b"abc");
        let _ = p.call("nope", b"");
        let t = mock.trace();
        assert_eq!(t.returned.len(), 4, "meta, init and two calls");
        let freed: Vec<(u32, u32)> = t.freed.iter().map(|&(p, l)| (p as u32, l as u32)).collect();
        assert_eq!(freed, t.returned);
    }

    #[test]
    fn empty_buffers_are_allocated_but_never_written() {
        let mock = Mock::default();
        let mut p = mock.load_with(WasmLimits::default(), b"").unwrap();
        call_ok(&mut p, "echo", b"");
        let t = mock.trace();
        assert_eq!(t.allocs, [0, 4, 0], "config, method, input");
        assert_eq!(t.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [4], "only the method name is written");
        assert_eq!(t.configs, [Vec::<u8>::new()]);
    }

    // --- status codes -----------------------------------------------------

    #[test]
    fn status_error_becomes_call_error_and_keeps_the_instance() {
        let mock = Mock::default();
        let mut p = mock.load().unwrap();
        match p.call("nope", b"") {
            Err(PluginError::Call { method, message }) => {
                assert_eq!(method, "nope");
                assert_eq!(message, "no method nope");
            }
            other => panic!("expected a call error, got {other:?}"),
        }
        assert_eq!(call_ok(&mut p, "count", b""), b"2", "same instance keeps counting");
        assert_eq!(mock.trace().instances, 1);
    }

    #[test]
    fn non_utf8_error_message_is_decoded_lossily() {
        let mock = with(|s| s.call = on_call(|_, _, _| status(STATUS_ERROR, &[0xff, b'x'])));
        let mut p = mock.load().unwrap();
        match p.call("m", b"") {
            Err(PluginError::Call { message, .. }) => assert_eq!(message, "\u{fffd}x"),
            other => panic!("expected a call error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_call_status_is_an_abi_violation_and_discards_the_instance() {
        for code in [plugwright_abi::STATUS_PANIC, 7, 255] {
            let mock = with(|s| {
                s.call = on_call(
                    move |m, _, n| if m == "bad" { status(code, b"payload") } else { ok(n.to_string().as_bytes()) },
                )
            });
            let mut p = mock.load().unwrap();
            assert_eq!(wasm_err(p.call("bad", b"").unwrap_err()), abi_err(&format!("unknown status {code}")));
            assert_eq!(call_ok(&mut p, "count", b""), b"1", "fresh instance after status {code}");
            assert_eq!(mock.trace().instances, 2);
        }
    }

    #[test]
    fn empty_result_buffer_is_an_abi_violation() {
        let mock = with(|s| s.call = on_call(|_, _, _| Reply::Buf(Vec::new())));
        let mut p = mock.load().unwrap();
        assert_eq!(wasm_err(p.call("m", b"").unwrap_err()), abi_err("empty result buffer"));

        let mock = with(|s| s.init = Reply::Buf(Vec::new()));
        assert_eq!(wasm_err(load_err(&mock)), abi_err("empty result buffer"));
    }

    #[test]
    fn call_result_must_be_i64() {
        for ret in [Some(WasmVal::I32(0)), None] {
            let mock = with(|s| s.call = on_call(move |_, _, _| Reply::Ret(ret)));
            let mut p = mock.load().unwrap();
            assert_eq!(wasm_err(p.call("m", b"").unwrap_err()), abi_err("`pw_call` must return i64"));
        }
    }

    // --- guest pointers and lengths ---------------------------------------

    #[test]
    fn out_of_bounds_or_overflowing_call_result_is_rejected_without_free() {
        for packed in [pack(MEM as u32 - 2, 4), pack(MEM as u32, 1), pack(u32::MAX - 1, 4), pack(u32::MAX, u32::MAX)] {
            let mock = with(|s| s.call = on_call(move |_, _, _| Reply::Packed(packed)));
            let mut p = mock.load_with(WasmLimits { max_memory_bytes: None, ..WasmLimits::default() }, b"").unwrap();
            let freed_before = mock.trace().freed.len();
            assert_eq!(wasm_err(p.call("m", b"").unwrap_err()), WasmError::OutOfBounds, "{:?}", unpack(packed));
            assert_eq!(mock.trace().freed.len(), freed_before);
        }
    }

    #[test]
    fn call_result_larger_than_memory_limit_is_rejected_before_reading() {
        let limits = WasmLimits { max_memory_bytes: Some(8), ..WasmLimits::default() };
        let mock = Mock::default();
        let mut p = mock.load_with(limits.clone(), b"").unwrap();
        assert_eq!(call_ok(&mut p, "echo", b"1234567"), b"1234567", "status + 7 bytes fits exactly");
        assert_eq!(
            wasm_err(p.call("echo", b"12345678").unwrap_err()),
            abi_err("`pw_call` returned 9 bytes, limit is 8")
        );

        // Larger than the guest memory: the length check wins over the bounds check.
        let mock = with(|s| s.call = on_call(|_, _, _| Reply::Packed(pack(0, 9))));
        let mut p = mock.load_with(limits, b"").unwrap();
        assert_eq!(wasm_err(p.call("m", b"").unwrap_err()), abi_err("`pw_call` returned 9 bytes, limit is 8"));
    }

    #[test]
    fn output_limit_follows_memory_limit_and_saturates_at_u32_max() {
        let mock = Mock::default();
        let max = |m: Option<usize>| {
            mock.load_with(WasmLimits { max_memory_bytes: m, ..WasmLimits::default() }, b"").unwrap().max_output()
        };
        assert_eq!(max(Some(4096)), 4096);
        assert_eq!(max(None), u32::MAX);
        assert_eq!(max(Some(usize::MAX)), u32::MAX);
    }

    #[test]
    fn init_result_out_of_bounds_or_over_limit_fails_load() {
        let mock = with(|s| s.init = Reply::Packed(pack(u32::MAX, 2)));
        assert_eq!(wasm_err(load_err(&mock)), WasmError::OutOfBounds);

        let mock = with(|s| s.init = Reply::Packed(pack(0, 100)));
        let limits = WasmLimits { max_memory_bytes: Some(10), ..WasmLimits::default() };
        let err = match mock.load_with(limits, b"") {
            Ok(_) => panic!("load unexpectedly succeeded"),
            Err(e) => wasm_err(e),
        };
        assert_eq!(err, abi_err("`pw_init` returned 100 bytes, limit is 10"));
    }

    #[test]
    fn alloc_returning_out_of_bounds_pointer_fails_on_write() {
        for ptr in [-4, MEM as i32 - 2, i32::MAX] {
            let mock = with(|s| s.alloc = Some(Reply::Ret(Some(WasmVal::I32(ptr)))));
            assert_eq!(wasm_err(load_err(&mock)), WasmError::OutOfBounds, "alloc returned {ptr}");
        }
    }

    #[test]
    fn alloc_must_return_i32() {
        for ret in [Some(WasmVal::I64(8)), None] {
            let mock = with(|s| s.alloc = Some(Reply::Ret(ret)));
            assert_eq!(wasm_err(load_err(&mock)), abi_err("`pw_alloc` must return i32"));
        }
    }

    #[test]
    fn alloc_trap_during_call_discards_the_instance() {
        let mock = Mock::default();
        let mut p = mock.load().unwrap();
        mock.edit(|s| s.alloc = Some(Reply::Fail(WasmError::MemoryLimit)));
        assert_eq!(wasm_err(p.call("echo", b"x").unwrap_err()), WasmError::MemoryLimit);
        mock.edit(|s| s.alloc = None);
        assert_eq!(call_ok(&mut p, "count", b""), b"1");
        assert_eq!(mock.trace().instances, 2);
    }

    // --- metadata ---------------------------------------------------------

    #[test]
    fn metadata_over_1024_bytes_is_rejected() {
        let mock = with(|s| s.meta = Reply::Packed(pack(0, 1025)));
        assert_eq!(wasm_err(load_err(&mock)), abi_err("`pw_meta` returned 1025 bytes, limit is 1024"));
    }

    #[test]
    fn metadata_of_exactly_1024_bytes_is_accepted() {
        let sdk = host_sdk_version().to_string();
        let fixed = meta("", "1.0.0", &sdk).len();
        let name = "n".repeat(1024 - fixed);
        let mock = with(|s| s.meta = Reply::Buf(meta(&name, "1.0.0", &sdk)));
        assert_eq!(mock.load().unwrap().info().name, name);
    }

    #[test]
    fn metadata_out_of_bounds_is_rejected() {
        for packed in [pack(MEM as u32 - 1, 2), pack(u32::MAX, 1)] {
            let mock = with(|s| s.meta = Reply::Packed(packed));
            assert_eq!(wasm_err(load_err(&mock)), WasmError::OutOfBounds);
        }
    }

    #[test]
    fn metadata_must_be_returned_as_i64() {
        for ret in [Some(WasmVal::I32(0)), None] {
            let mock = with(|s| s.meta = Reply::Ret(ret));
            assert_eq!(wasm_err(load_err(&mock)), abi_err("`pw_meta` must return i64"));
        }
    }

    #[test]
    fn malformed_metadata_is_rejected() {
        let sdk = host_sdk_version().to_string();
        let cases: [Vec<u8>; 4] = [
            Vec::new(),
            b"demo".to_vec(),
            b"demo\x001.0.0".to_vec(),
            format!("demo\x001.0.0\0{sdk}\0extra").into_bytes(),
        ];
        for bytes in cases {
            let mock = with(|s| s.meta = Reply::Buf(bytes.clone()));
            assert_eq!(
                wasm_err(load_err(&mock)),
                abi_err("metadata must be `name\\0version\\0sdk_version`"),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn non_utf8_metadata_is_rejected() {
        let mock = with(|s| s.meta = Reply::Buf(b"de\xffmo\x001.0.0\x000.1.0".to_vec()));
        assert_eq!(wasm_err(load_err(&mock)), abi_err("metadata is not UTF-8"));
    }

    #[test]
    fn invalid_versions_in_metadata_are_load_errors() {
        let sdk = host_sdk_version().to_string();
        for (version, sdk, prefix) in
            [("x", sdk.as_str(), "invalid plugin version `x`: "), ("1.0.0", "1.", "invalid SDK version `1.`: ")]
        {
            let mock = with(|s| s.meta = Reply::Buf(meta("demo", version, sdk)));
            match load_err(&mock) {
                PluginError::Load(msg) => assert!(msg.starts_with(prefix), "{msg}"),
                other => panic!("expected a load error, got {other:?}"),
            }
        }
    }

    #[test]
    fn incompatible_sdk_is_rejected() {
        let host = host_sdk_version();
        let mut sdks = vec![semver::Version::new(host.major + 1, 0, 0)];
        if host.major == 0 {
            sdks.push(semver::Version::new(0, host.minor + 1, 0));
        }
        for sdk in sdks {
            let mock = with(|s| s.meta = Reply::Buf(meta("demo", "1.0.0", &sdk.to_string())));
            match load_err(&mock) {
                PluginError::SdkMismatch { id, found, .. } => {
                    assert_eq!(id, "demo-id");
                    assert_eq!(found, sdk.to_string());
                }
                other => panic!("expected an SDK mismatch, got {other:?}"),
            }
        }
    }

    // --- ABI version and exports ------------------------------------------

    #[test]
    fn abi_version_mismatch_is_reported() {
        for (ret, found) in [(ABI_VERSION as i32 + 1, ABI_VERSION + 1), (0, 0), (-1, u32::MAX)] {
            let mock = with(|s| s.abi = Reply::Ret(Some(WasmVal::I32(ret))));
            match load_err(&mock) {
                PluginError::AbiMismatch { expected, found: f } => {
                    assert_eq!(expected, ABI_VERSION);
                    assert_eq!(f, found);
                }
                other => panic!("expected an ABI mismatch, got {other:?}"),
            }
        }
    }

    #[test]
    fn abi_version_must_be_i32() {
        for ret in [Some(WasmVal::I64(ABI_VERSION as i64)), None] {
            let mock = with(|s| s.abi = Reply::Ret(ret));
            assert_eq!(wasm_err(load_err(&mock)), abi_err("`pw_abi_version` must return i32"));
        }
    }

    #[test]
    fn missing_boot_exports_fail_load_with_their_name() {
        for export in [abi::EXPORT_ABI_VERSION, abi::EXPORT_META, abi::EXPORT_ALLOC, abi::EXPORT_FREE, abi::EXPORT_INIT]
        {
            let mock = with(|s| s.missing = vec![export]);
            assert_eq!(wasm_err(load_err(&mock)), WasmError::MissingExport(export.into()));
        }
    }

    #[test]
    fn missing_call_export_fails_the_call() {
        let mock = with(|s| s.missing = vec![abi::EXPORT_CALL]);
        let mut p = mock.load().unwrap();
        assert_eq!(wasm_err(p.call("m", b"").unwrap_err()), WasmError::MissingExport("pw_call".into()));
    }

    // --- init -------------------------------------------------------------

    #[test]
    fn init_error_status_fails_load_with_its_payload() {
        let mock = with(|s| s.init = status(STATUS_ERROR, b"bad config"));
        match load_err(&mock) {
            PluginError::Load(msg) => assert_eq!(msg, "bad config"),
            other => panic!("expected a load error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_init_status_also_fails_load() {
        let mock = with(|s| s.init = status(9, b"\xffhuh"));
        match load_err(&mock) {
            PluginError::Load(msg) => assert_eq!(msg, "\u{fffd}huh"),
            other => panic!("expected a load error, got {other:?}"),
        }
    }

    #[test]
    fn engine_compile_and_instantiate_errors_propagate() {
        let mock = with(|s| s.compile = Some(WasmError::Compile("bad magic".into())));
        assert_eq!(wasm_err(load_err(&mock)), WasmError::Compile("bad magic".into()));
        let mock = with(|s| s.instantiate = Some(WasmError::Instantiate("import denied".into())));
        assert_eq!(wasm_err(load_err(&mock)), WasmError::Instantiate("import denied".into()));
    }

    // --- trap recovery (D14) ----------------------------------------------

    #[test]
    fn poisoning_failure_discards_instance_and_next_call_reinstantiates() {
        let failures = [
            WasmError::Trap("unreachable".into()),
            WasmError::OutOfFuel,
            WasmError::Timeout,
            WasmError::MemoryLimit,
            WasmError::OutOfBounds,
            WasmError::Abi("bad".into()),
        ];
        for failure in failures {
            let f = failure.clone();
            let mock = with(move |s| {
                s.call = on_call(move |m, _, n| match m {
                    "fail" => Reply::Fail(f.clone()),
                    _ => ok(n.to_string().as_bytes()),
                })
            });
            let mut p = mock.load().unwrap();
            assert_eq!(call_ok(&mut p, "count", b""), b"1");
            assert_eq!(call_ok(&mut p, "count", b""), b"2");
            assert_eq!(wasm_err(p.call("fail", b"").unwrap_err()), failure);
            assert!(p.instance.is_none(), "{failure} must discard the instance");
            assert_eq!(call_ok(&mut p, "count", b""), b"1", "fresh state after {failure}");
            let t = mock.trace();
            assert_eq!(t.instances, 2);
            assert_eq!(t.configs, [b"cfg".to_vec(), b"cfg".to_vec()], "pw_init re-runs with the same config");
        }
    }

    #[test]
    fn begin_call_failure_discards_the_instance() {
        let mock = Mock::default();
        let mut p = mock.load().unwrap();
        call_ok(&mut p, "count", b"");
        mock.edit(|s| s.begin_call = Some(WasmError::Timeout));
        assert_eq!(wasm_err(p.call("count", b"").unwrap_err()), WasmError::Timeout);
        mock.edit(|s| s.begin_call = None);
        assert_eq!(call_ok(&mut p, "count", b""), b"1");
        assert_eq!(mock.trace().instances, 2);
    }

    #[test]
    fn failed_reinstantiation_is_retried_on_the_next_call() {
        let mock = Mock::default();
        let mut p = mock.load().unwrap();
        mock.edit(|s| s.begin_call = Some(WasmError::OutOfFuel));
        assert!(p.call("count", b"").is_err());
        mock.edit(|s| {
            s.begin_call = None;
            s.instantiate = Some(WasmError::Instantiate("boom".into()));
        });
        assert_eq!(wasm_err(p.call("count", b"").unwrap_err()), WasmError::Instantiate("boom".into()));
        assert!(p.instance.is_none());
        mock.edit(|s| s.instantiate = None);
        assert_eq!(call_ok(&mut p, "count", b""), b"1");
        assert_eq!(mock.trace().instances, 2);
    }

    #[test]
    fn failed_reinit_is_reported_and_retried() {
        let mock = Mock::default();
        let mut p = mock.load().unwrap();
        mock.edit(|s| {
            s.begin_call = Some(WasmError::Trap("x".into()));
        });
        assert!(p.call("count", b"").is_err());
        mock.edit(|s| {
            s.begin_call = None;
            s.init = status(STATUS_ERROR, b"no db");
        });
        assert!(matches!(p.call("count", b""), Err(PluginError::Load(m)) if m == "no db"));
        mock.edit(|s| s.init = ok(b""));
        assert_eq!(call_ok(&mut p, "count", b""), b"1");
        assert_eq!(mock.trace().instances, 3);
    }

    // --- limits -----------------------------------------------------------

    #[test]
    fn limits_deserialize_timeout_from_millis_and_default_the_rest() {
        let l: WasmLimits = serde_json::from_str(r#"{"timeout_ms": 250, "fuel": null}"#).unwrap();
        assert_eq!(l.timeout, Some(Duration::from_millis(250)));
        assert_eq!(l.fuel, None);
        assert_eq!(l.max_memory_bytes, WasmLimits::default().max_memory_bytes);

        let l: WasmLimits = serde_json::from_str(r#"{"timeout_ms": null}"#).unwrap();
        assert_eq!(l.timeout, None);
        let l: WasmLimits = serde_json::from_str("{}").unwrap();
        assert_eq!(l.timeout, Some(Duration::from_secs(5)));
    }

    #[test]
    fn limits_reject_unknown_fields_and_serialize_timeout_as_millis() {
        assert!(serde_json::from_str::<WasmLimits>(r#"{"timeout": 1}"#).is_err());
        let json = serde_json::to_value(WasmLimits::default()).unwrap();
        assert_eq!(json["timeout_ms"], 5000);
        let none = serde_json::to_value(WasmLimits { timeout: None, ..WasmLimits::default() }).unwrap();
        assert!(none["timeout_ms"].is_null());
    }

    // --- wasm_val_bridge! -------------------------------------------------

    #[derive(Debug, PartialEq)]
    enum V {
        I32(i32),
        I64(i64),
        F32(f32),
    }

    crate::wasm_val_bridge!(V);

    #[test]
    fn bridge_converts_args_to_engine_values() {
        assert_eq!(to_engine_vals(&[WasmVal::I32(-1), WasmVal::I64(1 << 40)]), [V::I32(-1), V::I64(1 << 40)]);
        assert!(to_engine_vals(&[]).is_empty());
    }

    #[test]
    fn bridge_takes_the_first_result() {
        assert_eq!(from_engine_result("f", &[V::I32(7), V::F32(0.5)]), Ok(Some(WasmVal::I32(7))));
        assert_eq!(from_engine_result("f", &[V::I64(-9)]), Ok(Some(WasmVal::I64(-9))));
        assert_eq!(from_engine_result("f", &[]), Ok(None));
    }

    #[test]
    fn bridge_rejects_unsupported_result_types() {
        assert_eq!(from_engine_result("f", &[V::F32(1.5)]), Err(abi_err("`f` returned unsupported value F32(1.5)")));
    }
}
