//! [`WasmEngine`] adapter for [wasmer](https://wasmer.io) (Cranelift backend).
//!
//! Lives outside the plugwright workspace: wasmer 7.5 pins `cranelift-codegen
//! =0.136.0` while wasmtime 49 needs `^0.136.2`, so the two cannot share one
//! dependency graph. The adapter itself needs only `plugwright-core` and
//! `plugwright-wasm`; a host that wants wasmer depends on this crate and on
//! `plugwright` with `default-features = false, features = ["wasmi"]` (or none).
//!
//! Limits:
//! * **fuel** — `wasmer-middlewares` metering, one point per operator;
//! * **memory** — every linear memory is wrapped so that `memory.grow` past the
//!   limit fails (the guest sees `-1`) and the failure is reported as
//!   [`WasmError::MemoryLimit`];
//! * **timeout** — the host waits for the worker until the deadline, then (on
//!   unix with the default `host-interrupt` feature) interrupts the store with
//!   wasmer's signal-based interrupter and reports [`WasmError::Timeout`]. If the
//!   worker still does not answer within [`GRACE`] (no interrupter on this
//!   platform, or a host import is running), it is abandoned: the call returns
//!   `Timeout` anyway, the instance is discarded, and fuel bounds how much CPU
//!   the detached thread may still burn.
//!
//! wasmer 7's `Store` is not `Send`, while [`WasmInstance`] must be. Each
//! instance therefore lives on its own worker thread and calls are forwarded
//! over a channel. A panic inside wasmer stays on that thread and surfaces as a
//! trap, which makes the manager discard the instance.
//!
//! ```no_run
//! # use std::sync::Arc;
//! # fn demo(mgr: &mut plugwright::PluginManager) {
//! mgr.register_engine(Arc::new(plugwright_wasmer::WasmerEngine::new()));
//! # }
//! ```

use std::fmt;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, channel, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use plugwright_core::logging::LogSink;
use plugwright_wasm::{MAX_GUEST_LOG, WasmEngine, WasmError, WasmInstance, WasmLimits, WasmModule, WasmVal};

plugwright_wasm::wasm_val_bridge!(Value);
use plugwright_abi::wasm::{EXPORT_MEMORY, IMPORT_LOG, IMPORT_MODULE};
use wasmer::sys::vm::{LinearMemory, VMMemory, VMMemoryDefinition, VMTable, VMTableDefinition};
use wasmer::sys::{BaseTunables, CompilerConfig, Cranelift, EngineBuilder, NativeEngineExt, Tunables};
use wasmer::{
    Engine, Function, FunctionEnv, FunctionEnvMut, Imports, Instance, Memory, MemoryError, MemoryStyle, MemoryType,
    Module, Pages, RuntimeError, Store, TableStyle, TableType, Value, WASM_PAGE_SIZE,
};
use wasmer_middlewares::Metering;
use wasmer_middlewares::metering::{MeteringPoints, get_remaining_points, set_remaining_points};

/// Fuel available to the start function, before the first `begin_call`.
const INIT_POINTS: u64 = 100_000_000;

/// How long an interrupted worker gets to answer before it is abandoned.
pub const GRACE: Duration = Duration::from_millis(200);

#[cfg(all(unix, feature = "host-interrupt"))]
type Interrupter = wasmer::Interrupter;

/// Stand-in where wasmer cannot interrupt a running guest.
#[cfg(not(all(unix, feature = "host-interrupt")))]
#[derive(Clone)]
struct Interrupter;

#[cfg(not(all(unix, feature = "host-interrupt")))]
impl Interrupter {
    fn interrupt(&self) {}
}

fn interrupter(_store: &Store) -> Option<Interrupter> {
    #[cfg(all(unix, feature = "host-interrupt"))]
    return Some(_store.interrupter());
    #[cfg(not(all(unix, feature = "host-interrupt")))]
    None
}

pub struct WasmerEngine {
    engine: Engine,
}

impl WasmerEngine {
    pub fn new() -> Self {
        let mut compiler = Cranelift::default();
        compiler.push_middleware(Arc::new(Metering::new(INIT_POINTS, |_| 1)));
        WasmerEngine { engine: EngineBuilder::new(compiler).engine().into() }
    }
}

impl Default for WasmerEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmEngine for WasmerEngine {
    fn name(&self) -> &str {
        "wasmer"
    }

    fn compile(&self, wasm: &[u8]) -> Result<Arc<dyn WasmModule>, WasmError> {
        let module = Module::new(&self.engine, wasm).map_err(|e| WasmError::Compile(e.to_string()))?;
        Ok(Arc::new(WrModule { engine: self.engine.clone(), module }))
    }
}

struct WrModule {
    engine: Engine,
    module: Module,
}

/// Engine tunables that wrap every guest memory in [`LimitedMemory`].
///
/// Tunables belong to an `Engine`, and an `Engine` clone shares compiled
/// artifacts but not tunables, so each instance gets a clone with its own limit.
struct LimitingTunables {
    base: BaseTunables,
    max_bytes: Option<usize>,
    hit: Arc<AtomicBool>,
}

impl LimitingTunables {
    fn wrap(
        &self,
        ty: &MemoryType,
        create: impl FnOnce() -> Result<VMMemory, MemoryError>,
    ) -> Result<VMMemory, MemoryError> {
        if ty.shared {
            return Err(MemoryError::Generic("shared memories are not supported".into()));
        }
        if let Some(max) = self.max_bytes
            && ty.minimum.bytes().0 > max
        {
            self.hit.store(true, Ordering::Relaxed);
            return Err(MemoryError::MinimumMemoryTooLarge {
                min_requested: ty.minimum,
                max_allowed: Pages((max / WASM_PAGE_SIZE) as u32),
            });
        }
        let inner = create()?;
        Ok(VMMemory::from_custom(Box::new(LimitedMemory { inner, max_bytes: self.max_bytes, hit: self.hit.clone() })
            as Box<dyn LinearMemory + Send + Sync>))
    }
}

impl Tunables for LimitingTunables {
    fn memory_style(&self, memory: &MemoryType) -> MemoryStyle {
        self.base.memory_style(memory)
    }

    fn table_style(&self, table: &TableType) -> TableStyle {
        self.base.table_style(table)
    }

    fn create_host_memory(&self, ty: &MemoryType, style: &MemoryStyle) -> Result<VMMemory, MemoryError> {
        self.wrap(ty, || self.base.create_host_memory(ty, style))
    }

    unsafe fn create_vm_memory(
        &self,
        ty: &MemoryType,
        style: &MemoryStyle,
        vm_definition_location: NonNull<VMMemoryDefinition>,
    ) -> Result<VMMemory, MemoryError> {
        // SAFETY: forwarded verbatim; the caller upholds the base contract.
        self.wrap(ty, || unsafe { self.base.create_vm_memory(ty, style, vm_definition_location) })
    }

    fn create_host_table(&self, ty: &TableType, style: &TableStyle) -> Result<VMTable, String> {
        self.base.create_host_table(ty, style)
    }

    unsafe fn create_vm_table(
        &self,
        ty: &TableType,
        style: &TableStyle,
        vm_definition_location: NonNull<VMTableDefinition>,
    ) -> Result<VMTable, String> {
        // SAFETY: forwarded verbatim; the caller upholds the base contract.
        unsafe { self.base.create_vm_table(ty, style, vm_definition_location) }
    }
}

/// A linear memory that refuses to grow past `max_bytes`.
struct LimitedMemory {
    inner: VMMemory,
    max_bytes: Option<usize>,
    hit: Arc<AtomicBool>,
}

impl fmt::Debug for LimitedMemory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LimitedMemory").field("max_bytes", &self.max_bytes).finish_non_exhaustive()
    }
}

impl LimitedMemory {
    fn check(&self, new_bytes: u64) -> Result<(), MemoryError> {
        match self.max_bytes {
            Some(max) if new_bytes > max as u64 => {
                self.hit.store(true, Ordering::Relaxed);
                Err(MemoryError::Generic(format!("memory limit of {max} bytes reached")))
            }
            _ => Ok(()),
        }
    }

    fn rewrap(&self, inner: Box<dyn LinearMemory + Send + Sync>) -> Box<dyn LinearMemory + Send + Sync> {
        Box::new(LimitedMemory { inner: VMMemory(inner), max_bytes: self.max_bytes, hit: self.hit.clone() })
    }
}

impl LinearMemory for LimitedMemory {
    fn ty(&self) -> MemoryType {
        self.inner.ty()
    }

    fn size(&self) -> Pages {
        self.inner.size()
    }

    fn style(&self) -> MemoryStyle {
        self.inner.style()
    }

    fn grow(&mut self, delta: Pages) -> Result<Pages, MemoryError> {
        let new_pages = u64::from(self.inner.size().0) + u64::from(delta.0);
        if let Err(e) = self.check(new_pages * WASM_PAGE_SIZE as u64) {
            tracing::debug!(current = self.inner.size().0, delta = delta.0, "wasmer: memory.grow denied");
            return Err(e);
        }
        self.inner.grow(delta)
    }

    fn grow_at_least(&mut self, min_size: u64) -> Result<(), MemoryError> {
        self.check(min_size)?;
        self.inner.grow_at_least(min_size)
    }

    fn reset(&mut self) -> Result<(), MemoryError> {
        self.inner.reset()
    }

    fn vmmemory(&self) -> NonNull<VMMemoryDefinition> {
        self.inner.vmmemory()
    }

    fn try_clone(&self) -> Result<Box<dyn LinearMemory + Send + Sync + 'static>, MemoryError> {
        Ok(self.rewrap(LinearMemory::try_clone(&self.inner)?))
    }

    fn copy(&self) -> Result<Box<dyn LinearMemory + Send + Sync + 'static>, MemoryError> {
        Ok(self.rewrap(self.inner.copy()?))
    }
}

struct Env {
    log: LogSink,
    memory: Option<Memory>,
}

impl WasmModule for WrModule {
    fn instantiate(&self, limits: &WasmLimits, log: LogSink) -> Result<Box<dyn WasmInstance>, WasmError> {
        let timeout = limits.timeout;
        let (engine, module, limits) = (self.engine.clone(), self.module.clone(), limits.clone());
        let timed_out = Arc::new(AtomicBool::new(false));
        let flag = timed_out.clone();
        let (tx, rx) = channel::<Job>();
        let (ready_tx, ready_rx) = sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("plugwright-wasmer".into())
            .spawn(move || match Worker::new(engine, &module, limits, log, flag) {
                Ok(mut w) => {
                    let _ = ready_tx.send(Ok(interrupter(&w.store)));
                    w.serve(rx);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| WasmError::Instantiate(format!("cannot spawn wasmer worker: {e}")))?;
        let ready = ready_rx.recv().unwrap_or_else(|_| Err(worker_died()));
        let mut inst =
            WrInstance { tx: Some(tx), worker: Some(worker), interrupter: None, timeout, deadline: None, timed_out };
        inst.interrupter = ready?;
        Ok(Box::new(inst))
    }
}

type Job = Box<dyn FnOnce(&mut Worker) + Send>;

fn worker_died() -> WasmError {
    WasmError::Trap("wasmer worker thread died".into())
}

/// Owns the non-`Send` wasmer state; runs on a dedicated thread.
struct Worker {
    store: Store,
    instance: Instance,
    memory: Memory,
    limits: WasmLimits,
    hit: Arc<AtomicBool>,
    /// Set by the host before it interrupts the store.
    timed_out: Arc<AtomicBool>,
}

impl Worker {
    fn new(
        base: Engine,
        module: &Module,
        limits: WasmLimits,
        log: LogSink,
        timed_out: Arc<AtomicBool>,
    ) -> Result<Self, WasmError> {
        let hit = Arc::new(AtomicBool::new(false));
        let mut engine = base;
        engine.set_tunables(LimitingTunables {
            base: BaseTunables::new(),
            max_bytes: limits.max_memory_bytes,
            hit: hit.clone(),
        });
        let mut store = Store::new(engine);
        let env = FunctionEnv::new(&mut store, Env { log, memory: None });
        let log_fn = Function::new_typed_with_env(
            &mut store,
            &env,
            |mut ctx: FunctionEnvMut<'_, Env>, level: i32, ptr: i32, len: i32| {
                let (data, store) = ctx.data_and_store_mut();
                let Some(mem) = data.memory.clone() else { return };
                let mut buf = vec![0u8; (len as u32).min(MAX_GUEST_LOG) as usize];
                if mem.view(&store).read(u64::from(ptr as u32), &mut buf).is_ok() {
                    (data.log)(level as u32, &String::from_utf8_lossy(&buf));
                }
            },
        );
        // Only `plugwright.log` is provided; any other import fails instantiation.
        let mut imports = Imports::new();
        imports.define(IMPORT_MODULE, IMPORT_LOG, log_fn);
        let instance = Instance::new(&mut store, module, &imports).map_err(|e| {
            if hit.swap(false, Ordering::Relaxed) {
                WasmError::MemoryLimit
            } else {
                WasmError::Instantiate(e.to_string())
            }
        })?;
        let memory = instance
            .exports
            .get_memory(EXPORT_MEMORY)
            .map_err(|_| WasmError::MissingExport(EXPORT_MEMORY.into()))?
            .clone();
        env.as_mut(&mut store).memory = Some(memory.clone());
        Ok(Worker { store, instance, memory, limits, hit, timed_out })
    }

    fn serve(&mut self, jobs: Receiver<Job>) {
        for job in jobs {
            job(self);
        }
    }

    fn map_err(&mut self, e: RuntimeError) -> WasmError {
        if self.hit.swap(false, Ordering::Relaxed) {
            return WasmError::MemoryLimit;
        }
        if self.timed_out.load(Ordering::Relaxed) {
            return WasmError::Timeout;
        }
        if matches!(get_remaining_points(&mut self.store, &self.instance), MeteringPoints::Exhausted) {
            return WasmError::OutOfFuel;
        }
        tracing::debug!(error = ?e, "wasmer trap");
        WasmError::Trap(e.message())
    }

    fn begin_call(&mut self) {
        set_remaining_points(&mut self.store, &self.instance, self.limits.fuel.unwrap_or(u64::MAX));
    }

    fn call(&mut self, func: &str, args: &[WasmVal]) -> Result<Option<WasmVal>, WasmError> {
        let f = self.instance.exports.get_function(func).map_err(|_| WasmError::MissingExport(func.into()))?.clone();
        let params = to_engine_vals(args);
        let results = match f.call(&mut self.store, &params) {
            Ok(r) => r,
            Err(e) => return Err(self.map_err(e)),
        };
        from_engine_result(func, &results)
    }

    fn read(&self, ptr: u32, len: u32) -> Result<Vec<u8>, WasmError> {
        // Bounds first: `len` comes from the guest and must not size an allocation.
        let view = self.memory.view(&self.store);
        if u64::from(ptr) + u64::from(len) > view.data_size() {
            return Err(WasmError::OutOfBounds);
        }
        let mut buf = vec![0u8; len as usize];
        view.read(u64::from(ptr), &mut buf).map_err(|_| WasmError::OutOfBounds)?;
        Ok(buf)
    }

    fn write(&self, ptr: u32, data: &[u8]) -> Result<(), WasmError> {
        self.memory.view(&self.store).write(u64::from(ptr), data).map_err(|_| WasmError::OutOfBounds)
    }
}

/// `Send` handle to a [`Worker`].
struct WrInstance {
    tx: Option<Sender<Job>>,
    worker: Option<JoinHandle<()>>,
    interrupter: Option<Interrupter>,
    timeout: Option<Duration>,
    /// Wall-clock deadline of the current call sequence.
    deadline: Option<Instant>,
    timed_out: Arc<AtomicBool>,
}

impl WrInstance {
    /// Run `f` on the worker thread and wait for its result, at most until the
    /// deadline (plus [`GRACE`] after interrupting the guest).
    fn run<R: Send + 'static>(
        &mut self,
        f: impl FnOnce(&mut Worker) -> Result<R, WasmError> + Send + 'static,
    ) -> Result<R, WasmError> {
        let (reply_tx, reply_rx): (SyncSender<Result<R, WasmError>>, _) = sync_channel(1);
        let tx = self.tx.as_ref().ok_or_else(worker_died)?;
        tx.send(Box::new(move |w: &mut Worker| {
            let _ = reply_tx.send(f(w));
        }))
        .map_err(|_| worker_died())?;
        let Some(deadline) = self.deadline else {
            return reply_rx.recv().unwrap_or_else(|_| Err(worker_died()));
        };
        match reply_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(r) => return r,
            Err(RecvTimeoutError::Disconnected) => return Err(worker_died()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        self.timed_out.store(true, Ordering::Relaxed);
        if let Some(i) = &self.interrupter {
            // A signal can be missed if it lands just before the guest is
            // entered (see `Store::interrupter`), so retry within the grace.
            let until = Instant::now() + GRACE;
            while Instant::now() < until {
                i.interrupt();
                match reply_rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(_) | Err(RecvTimeoutError::Disconnected) => return Err(WasmError::Timeout),
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        }
        tracing::warn!("wasmer: guest did not stop after the deadline; abandoning its worker thread");
        self.tx = None;
        drop(self.worker.take()); // detach: joining could block forever
        Err(WasmError::Timeout)
    }
}

impl WasmInstance for WrInstance {
    fn begin_call(&mut self) -> Result<(), WasmError> {
        self.deadline = self.timeout.map(|t| Instant::now() + t);
        self.timed_out.store(false, Ordering::Relaxed);
        self.run(|w| {
            w.begin_call();
            Ok(())
        })
    }

    fn call(&mut self, func: &str, args: &[WasmVal]) -> Result<Option<WasmVal>, WasmError> {
        let (func, args) = (func.to_owned(), args.to_vec());
        self.run(move |w| w.call(&func, &args))
    }

    fn read(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>, WasmError> {
        self.run(move |w| w.read(ptr, len))
    }

    fn write(&mut self, ptr: u32, data: &[u8]) -> Result<(), WasmError> {
        let data = data.to_vec();
        self.run(move |w| w.write(ptr, &data))
    }
}

impl Drop for WrInstance {
    fn drop(&mut self) {
        // Closing the channel ends `serve`; the worker then drops the store.
        drop(self.tx.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(module (memory (export "memory") 1))`.
    const ONE_PAGE: &[u8] = b"\0asm\x01\0\0\0\x05\x03\x01\x00\x01\x07\x0a\x01\x06memory\x02\x00";

    #[test]
    fn guest_memory_access_is_bounds_checked_before_allocating() {
        let module = WasmerEngine::new().compile(ONE_PAGE).unwrap();
        let mut inst = module.instantiate(&WasmLimits::default(), Arc::new(|_: u32, _: &str| {})).unwrap();
        inst.write(65_532, b"tail").unwrap();
        assert_eq!(inst.read(65_532, 4).unwrap(), b"tail");
        assert_eq!(inst.read(65_536, 0).unwrap(), b"");
        // A guest-supplied length near u32::MAX must fail without allocating it.
        for (ptr, len) in [(65_533, 4), (0, u32::MAX), (u32::MAX, 1), (65_537, 0)] {
            assert!(matches!(inst.read(ptr, len), Err(WasmError::OutOfBounds)), "read({ptr}, {len})");
        }
        assert!(matches!(inst.write(65_533, b"tail"), Err(WasmError::OutOfBounds)));
    }
}
