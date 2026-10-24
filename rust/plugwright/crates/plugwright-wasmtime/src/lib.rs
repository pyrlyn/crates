//! [`WasmEngine`] adapter for wasmtime: fuel, epoch-based wall-clock timeout,
//! memory limiter, stack limit.

use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use plugwright_abi::wasm::{EXPORT_MEMORY, IMPORT_HOST_CALL, IMPORT_HOST_TAKE, IMPORT_LOG, IMPORT_MODULE};
use wasmtime::{
    Caller, Config, Engine, Extern, Instance, Linker, Memory, Module, ResourceLimiter, Store, Trap, UpdateDeadline, Val,
};

use plugwright_core::logging::LogSink;
use plugwright_wasm::{
    HostCalls, Imports, MAX_GUEST_LOG, WasmEngine, WasmError, WasmInstance, WasmLimits, WasmModule, WasmVal,
    guest_bytes, put_guest_bytes, stored,
};

plugwright_wasm::wasm_val_bridge!(Val);

/// Epoch tick; the timeout resolution.
const TICK: Duration = Duration::from_millis(10);

/// Increments the engine epoch until dropped, from the first instantiation
/// on. Shared by the engine, every module compiled on it and every instance,
/// so a plugin's wall-clock timeout keeps working after the
/// [`WasmtimeEngine`] that compiled it is gone.
///
/// The thread starts lazily: on aarch64 Linux the first code wasmtime
/// publishes in a process registers a `membarrier`, which is cheap in a
/// single-threaded process and costs an RCU grace period (~10 ms) once other
/// threads exist. Compiling before the ticker exists keeps the cheap path for
/// hosts that have no threads of their own.
struct Ticker {
    engine: Engine,
    running: Mutex<Option<Running>>,
}

struct Running {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl Ticker {
    fn new(engine: Engine) -> Self {
        Ticker { engine, running: Mutex::new(None) }
    }

    /// Start the thread unless it already runs.
    fn ensure_running(&self) -> Result<(), WasmError> {
        let mut running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        if running.is_some() {
            return Ok(());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (s, engine) = (Arc::clone(&stop), self.engine.clone());
        let handle = std::thread::Builder::new()
            .name("plugwright-wasmtime-epoch".into())
            .spawn(move || {
                let mut clock = EpochClock::new(Instant::now());
                while !s.load(Ordering::Acquire) {
                    let (ticks, wait) = clock.advance(Instant::now());
                    for _ in 0..ticks {
                        engine.increment_epoch();
                    }
                    // Parked rather than asleep, so `drop` does not wait out a tick.
                    std::thread::park_timeout(wait);
                }
            })
            .map_err(|e| WasmError::Instantiate(format!("cannot start epoch thread: {e}")))?;
        *running = Some(Running { stop, handle });
        Ok(())
    }
}

/// Maps wall time to epoch ticks. Deadlines are counted in ticks, so a
/// wakeup that comes late (a loaded machine, timer coalescing) must catch up
/// on every tick it missed, or a 300 ms timeout stretches to seconds.
struct EpochClock {
    start: Instant,
    ticks: u64,
}

impl EpochClock {
    fn new(start: Instant) -> Self {
        EpochClock { start, ticks: 0 }
    }

    /// Ticks to add at `now`, and how long until the next one is due.
    fn advance(&mut self, now: Instant) -> (u64, Duration) {
        let elapsed = now.saturating_duration_since(self.start).as_nanos();
        let tick = TICK.as_nanos();
        let due = (elapsed / tick) as u64;
        let add = due.saturating_sub(self.ticks);
        self.ticks = self.ticks.max(due);
        (add, Duration::from_nanos((tick - elapsed % tick) as u64))
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        let running = self.running.get_mut().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(Running { stop, handle }) = running {
            stop.store(true, Ordering::Release);
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

pub struct WasmtimeEngine {
    engine: Engine,
    ticker: Arc<Ticker>,
}

impl WasmtimeEngine {
    pub fn new() -> Result<Self, WasmError> {
        let mut config = Config::new();
        config.max_wasm_stack(512 << 10);
        Self::with_config(config)
    }

    /// Use a custom wasmtime `Config`. Fuel and epoch interruption are forced on.
    pub fn with_config(mut config: Config) -> Result<Self, WasmError> {
        config.consume_fuel(true).epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|e| WasmError::Compile(e.to_string()))?;
        Ok(WasmtimeEngine { ticker: Arc::new(Ticker::new(engine.clone())), engine })
    }

    fn module(&self, module: Module) -> Arc<dyn WasmModule> {
        Arc::new(WtModule { engine: self.engine.clone(), module, ticker: Arc::clone(&self.ticker) })
    }
}

impl WasmEngine for WasmtimeEngine {
    fn name(&self) -> &str {
        "wasmtime"
    }

    fn compile(&self, wasm: &[u8]) -> Result<Arc<dyn WasmModule>, WasmError> {
        let module = Module::new(&self.engine, wasm).map_err(|e| WasmError::Compile(format!("{e:#}")))?;
        Ok(self.module(module))
    }

    /// Cranelift output is kept as `<sha256>-<engine compatibility>.cwasm`:
    /// loading it takes a read and a relocation instead of a compile.
    fn compile_cached(&self, wasm: &[u8], key: &str, dir: &Path) -> Result<Arc<dyn WasmModule>, WasmError> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.engine.precompile_compatibility_hash().hash(&mut h);
        let file = dir.join(format!("{key}-{:016x}.cwasm", h.finish()));
        if let Some(code) = stored::read(&file) {
            // SAFETY: `code` is intact (sha256-checked) output of
            // `Module::serialize` for an engine with this compatibility hash,
            // which wasmtime checks again. Nothing else validates it: the
            // cache directory is trusted like the native libraries the host
            // loads from the same cache.
            match unsafe { Module::deserialize(&self.engine, &code) } {
                Ok(module) => return Ok(self.module(module)),
                Err(e) => tracing::warn!(file = %file.display(), error = %e, "stale compiled module; recompiling"),
            }
        }
        let module = Module::new(&self.engine, wasm).map_err(|e| WasmError::Compile(format!("{e:#}")))?;
        match module.serialize() {
            Ok(code) => {
                if let Err(e) = stored::write(&file, &code) {
                    tracing::warn!(file = %file.display(), error = %e, "cannot keep compiled module");
                }
            }
            Err(e) => tracing::warn!(error = %e, "cannot serialize compiled module"),
        }
        Ok(self.module(module))
    }
}

struct WtModule {
    engine: Engine,
    module: Module,
    ticker: Arc<Ticker>,
}

/// Memory limiter that remembers whether it refused a growth request, so the
/// resulting trap can be reported as [`WasmError::MemoryLimit`].
struct Limiter {
    max_memory: Option<usize>,
    hit: bool,
}

impl ResourceLimiter for Limiter {
    fn memory_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        if self.max_memory.is_some_and(|m| desired > m) {
            self.hit = true;
            return Ok(false);
        }
        Ok(true)
    }

    fn table_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= 1_000_000)
    }
}

struct State {
    limiter: Limiter,
    log: LogSink,
    host: HostCalls,
    /// Wall-clock end of the current call, when it has a timeout.
    deadline: Option<Instant>,
}

/// Ticks only approximate time (the ticker catches up in bursts after a
/// stall), so a reached epoch deadline is confirmed on the real clock.
fn on_epoch_deadline(deadline: Option<Instant>, now: Instant) -> UpdateDeadline {
    match deadline {
        Some(d) if now < d => {
            let left = (d - now).as_nanos().div_ceil(TICK.as_nanos());
            UpdateDeadline::Continue(u64::try_from(left).unwrap_or(u64::MAX).max(1))
        }
        _ => UpdateDeadline::Interrupt,
    }
}

impl WasmModule for WtModule {
    fn instantiate(&self, limits: &WasmLimits, imports: &Imports) -> Result<Box<dyn WasmInstance>, WasmError> {
        self.ticker.ensure_running()?;
        let mut store = Store::new(
            &self.engine,
            State {
                limiter: Limiter { max_memory: limits.max_memory_bytes, hit: false },
                log: Arc::clone(&imports.log),
                host: HostCalls::new(imports.host.clone()),
                deadline: None,
            },
        );
        store.limiter(|s| &mut s.limiter);
        store.epoch_deadline_callback(|ctx| Ok(on_epoch_deadline(ctx.data().deadline, Instant::now())));
        let mut linker = Linker::new(&self.engine);
        linker
            .func_wrap(IMPORT_MODULE, IMPORT_LOG, |mut caller: Caller<'_, State>, level: i32, ptr: i32, len: i32| {
                let Some(Extern::Memory(mem)) = caller.get_export(EXPORT_MEMORY) else { return };
                let mut buf = vec![0u8; (len as u32).min(MAX_GUEST_LOG) as usize];
                if mem.read(&caller, ptr as u32 as usize, &mut buf).is_ok() {
                    (caller.data().log)(level as u32, &String::from_utf8_lossy(&buf));
                }
            })
            .and_then(|l| {
                l.func_wrap(
                    IMPORT_MODULE,
                    IMPORT_HOST_CALL,
                    |mut caller: Caller<'_, State>, np: i32, nl: i32, ip: i32, il: i32| -> i32 {
                        let Some(Extern::Memory(mem)) = caller.get_export(EXPORT_MEMORY) else { return -1 };
                        let (data, state) = mem.data_and_store_mut(&mut caller);
                        state.host.call(|p, l| guest_bytes(data, p, l), np, nl, ip, il)
                    },
                )
            })
            .and_then(|l| {
                l.func_wrap(
                    IMPORT_MODULE,
                    IMPORT_HOST_TAKE,
                    |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> i32 {
                        let Some(Extern::Memory(mem)) = caller.get_export(EXPORT_MEMORY) else { return -1 };
                        let (data, state) = mem.data_and_store_mut(&mut caller);
                        state.host.take(|p, d| put_guest_bytes(data, p, d), ptr, len)
                    },
                )
            })
            .map_err(|e| WasmError::Instantiate(e.to_string()))?;
        let mut inst = WtInstance {
            store,
            instance: None,
            memory: None,
            limits: limits.clone(),
            _ticker: Arc::clone(&self.ticker),
        };
        // A start function may run during instantiation: budget it like a call.
        inst.begin_call()?;
        let instance = linker.instantiate(&mut inst.store, &self.module).map_err(|e| inst.map_err(e))?;
        let memory = instance
            .get_memory(&mut inst.store, EXPORT_MEMORY)
            .ok_or_else(|| WasmError::MissingExport(EXPORT_MEMORY.into()))?;
        inst.instance = Some(instance);
        inst.memory = Some(memory);
        Ok(Box::new(inst))
    }
}

struct WtInstance {
    store: Store<State>,
    instance: Option<Instance>,
    memory: Option<Memory>,
    limits: WasmLimits,
    /// Its deadline only passes while the ticker runs.
    _ticker: Arc<Ticker>,
}

impl WtInstance {
    fn map_err(&mut self, e: wasmtime::Error) -> WasmError {
        if std::mem::take(&mut self.store.data_mut().limiter.hit) {
            return WasmError::MemoryLimit;
        }
        match e.downcast_ref::<Trap>() {
            Some(Trap::OutOfFuel) => WasmError::OutOfFuel,
            Some(Trap::Interrupt) => WasmError::Timeout,
            Some(trap) => {
                tracing::debug!(backtrace = %format!("{e:?}"), "wasm trap");
                // wasmtime's Display already starts with "wasm trap: ", which `WasmError::Trap` adds too.
                let msg = trap.to_string();
                WasmError::Trap(msg.strip_prefix("wasm trap: ").unwrap_or(&msg).to_owned())
            }
            None => WasmError::Trap(e.to_string().lines().next().unwrap_or_default().to_owned()),
        }
    }

    fn memory(&self) -> Result<Memory, WasmError> {
        self.memory.ok_or_else(|| WasmError::MissingExport(EXPORT_MEMORY.into()))
    }
}

impl WasmInstance for WtInstance {
    fn begin_call(&mut self) -> Result<(), WasmError> {
        self.store.set_fuel(self.limits.fuel.unwrap_or(u64::MAX)).map_err(|e| WasmError::Instantiate(e.to_string()))?;
        let ticks = match self.limits.timeout {
            Some(t) => (t.as_millis() / TICK.as_millis()).max(1) as u64,
            None => u64::MAX / 2,
        };
        self.store.set_epoch_deadline(ticks);
        self.store.data_mut().deadline = self.limits.timeout.map(|t| Instant::now() + t);
        Ok(())
    }

    fn call(&mut self, func: &str, args: &[WasmVal]) -> Result<Option<WasmVal>, WasmError> {
        let instance = self.instance.ok_or_else(|| WasmError::MissingExport(func.into()))?;
        let f = instance.get_func(&mut self.store, func).ok_or_else(|| WasmError::MissingExport(func.into()))?;
        let params = to_engine_vals(args);
        let n = f.ty(&self.store).results().len();
        let mut results = vec![Val::I32(0); n];
        if let Err(e) = f.call(&mut self.store, &params, &mut results) {
            return Err(self.map_err(e));
        }
        from_engine_result(func, &results)
    }

    fn read(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>, WasmError> {
        // Bounds first: `len` comes from the guest and must not size an allocation.
        let (ptr, len) = (ptr as usize, len as usize);
        let data = self.memory()?.data(&self.store);
        ptr.checked_add(len).and_then(|end| data.get(ptr..end)).map(<[u8]>::to_vec).ok_or(WasmError::OutOfBounds)
    }

    fn write(&mut self, ptr: u32, data: &[u8]) -> Result<(), WasmError> {
        self.memory()?.write(&mut self.store, ptr as usize, data).map_err(|_| WasmError::OutOfBounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_wakeup_catches_up_on_every_missed_tick() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut c = EpochClock::new(t0);
        assert_eq!(c.advance(ms(5)), (0, Duration::from_millis(5)));
        assert_eq!(c.advance(ms(10)), (1, TICK));
        // Asleep for 45 ms instead of 10: all four missed ticks at once.
        assert_eq!(c.advance(ms(55)), (4, Duration::from_millis(5)));
        assert_eq!(c.advance(ms(55)), (0, Duration::from_millis(5)), "never counted twice");
        // So a 300 ms deadline (30 ticks) passes after 300 ms however the wakeups fall.
        assert_eq!(c.advance(ms(300)).0, 30 - 5);
        assert_eq!(c.advance(t0).0, 0, "time never runs backwards");
    }

    #[test]
    fn a_reached_epoch_deadline_stops_only_once_the_wall_clock_has() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        // A catch-up burst reached the deadline 95 ms early: run 10 more ticks.
        assert!(matches!(on_epoch_deadline(Some(ms(100)), ms(5)), UpdateDeadline::Continue(10)));
        assert!(matches!(on_epoch_deadline(Some(ms(100)), ms(99)), UpdateDeadline::Continue(1)));
        assert!(matches!(on_epoch_deadline(Some(ms(100)), ms(100)), UpdateDeadline::Interrupt));
        assert!(matches!(on_epoch_deadline(None, ms(1)), UpdateDeadline::Interrupt));
    }

    /// `(module (memory (export "memory") 1))`.
    const ONE_PAGE: &[u8] = b"\0asm\x01\0\0\0\x05\x03\x01\x00\x01\x07\x0a\x01\x06memory\x02\x00";

    #[test]
    fn guest_memory_access_is_bounds_checked_before_allocating() {
        let module = WasmtimeEngine::new().unwrap().compile(ONE_PAGE).unwrap();
        let mut inst =
            module.instantiate(&WasmLimits::default(), &Imports::new(Arc::new(|_: u32, _: &str| {}))).unwrap();
        inst.write(65_532, b"tail").unwrap();
        assert_eq!(inst.read(65_532, 4).unwrap(), b"tail");
        assert_eq!(inst.read(65_536, 0).unwrap(), b"");
        // A guest-supplied length near u32::MAX must fail without allocating it.
        for (ptr, len) in [(65_533, 4), (0, u32::MAX), (u32::MAX, 1), (65_537, 0)] {
            assert!(matches!(inst.read(ptr, len), Err(WasmError::OutOfBounds)), "read({ptr}, {len})");
        }
        assert!(matches!(inst.write(65_533, b"tail"), Err(WasmError::OutOfBounds)));
    }

    #[test]
    fn compiled_code_is_kept_and_damaged_code_is_recompiled() {
        let dir = tempfile::tempdir().unwrap();
        let engine = WasmtimeEngine::new().unwrap();
        let usable = |m: Arc<dyn WasmModule>| {
            let mut inst =
                m.instantiate(&WasmLimits::default(), &Imports::new(Arc::new(|_: u32, _: &str| {}))).unwrap();
            inst.write(0, b"ok").unwrap();
        };
        usable(engine.compile_cached(ONE_PAGE, "k", dir.path()).unwrap());
        let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.path()).collect();
        let [file] = files.as_slice() else { panic!("{files:?}") };
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("k-") && name.ends_with(".cwasm"), "{name}");
        let code = stored::read(file).expect("intact");
        assert!(code.starts_with(b"\x7fELF"), "precompiled object");
        usable(engine.compile_cached(ONE_PAGE, "k", dir.path()).unwrap());

        // Damaged: ignored and rewritten; foreign but intact: rejected by wasmtime and rewritten.
        std::fs::write(file, b"garbage").unwrap();
        usable(engine.compile_cached(ONE_PAGE, "k", dir.path()).unwrap());
        assert_eq!(stored::read(file).as_deref(), Some(&code[..]));
        stored::write(file, b"\x7fELF not really").unwrap();
        usable(engine.compile_cached(ONE_PAGE, "k", dir.path()).unwrap());
        assert_eq!(stored::read(file).as_deref(), Some(&code[..]));
        // Another configuration gets a file of its own.
        let mut config = Config::new();
        config.max_wasm_stack(256 << 10).cranelift_opt_level(wasmtime::OptLevel::None);
        let other = WasmtimeEngine::with_config(config).unwrap();
        usable(other.compile_cached(ONE_PAGE, "k", dir.path()).unwrap());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn the_epoch_thread_starts_on_first_instantiation_and_outlives_the_engine() {
        let engine = WasmtimeEngine::new().unwrap();
        let ticker = Arc::clone(&engine.ticker);
        let running = || ticker.running.lock().unwrap().is_some();
        let module = engine.compile(ONE_PAGE).unwrap();
        assert!(!running(), "compiling needs no thread");
        let instance =
            module.instantiate(&WasmLimits::default(), &Imports::new(Arc::new(|_: u32, _: &str| {}))).unwrap();
        assert!(running());
        drop((engine, module));
        assert_eq!(Arc::strong_count(&ticker), 2, "the instance keeps the ticker");
        drop(instance);
        assert_eq!(Arc::strong_count(&ticker), 1);
    }
}
