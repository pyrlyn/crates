//! [`WasmEngine`] adapter for wasmi (interpreter): fuel, memory and wall-clock
//! limits.
//!
//! wasmi has no asynchronous interruption, but a call that runs out of fuel
//! can be *resumed*. With a `timeout`, fuel is handed out in [`FUEL_SLICE`]
//! portions; between slices the deadline is checked, so the guest is stopped
//! within one slice (well under a millisecond) of the deadline.

use std::sync::Arc;
use std::time::Instant;

use plugwright_abi::wasm::{EXPORT_MEMORY, IMPORT_HOST_CALL, IMPORT_HOST_TAKE, IMPORT_LOG, IMPORT_MODULE};
use wasmi::{
    Caller, CompilationMode, Config, Engine, Extern, Instance, Linker, Memory, Module, ResourceLimiter, ResumableCall,
    Store, TrapCode, Val,
};

use wasmi_core::LimiterError;

use plugwright_core::logging::LogSink;
use plugwright_wasm::{
    HostCalls, Imports, MAX_GUEST_LOG, WasmEngine, WasmError, WasmInstance, WasmLimits, WasmModule, WasmVal,
    guest_bytes, put_guest_bytes,
};

plugwright_wasm::wasm_val_bridge!(Val);

/// Fuel granted per slice when a wall-clock timeout is set.
const FUEL_SLICE: u64 = 100_000;

pub struct WasmiEngine {
    engine: Engine,
}

impl WasmiEngine {
    pub fn new() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        // Lazy translation charges fuel on first use, and that charge cannot be
        // resumed: a fuel slice could run dry mid-compilation. Pay it up front.
        config.compilation_mode(CompilationMode::Eager);
        WasmiEngine { engine: Engine::new(&config) }
    }
}

impl Default for WasmiEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmEngine for WasmiEngine {
    fn name(&self) -> &str {
        "wasmi"
    }

    fn compile(&self, wasm: &[u8]) -> Result<Arc<dyn WasmModule>, WasmError> {
        let module = Module::new(&self.engine, wasm).map_err(|e| WasmError::Compile(e.to_string()))?;
        Ok(Arc::new(WiModule { engine: self.engine.clone(), module }))
    }
}

struct WiModule {
    engine: Engine,
    module: Module,
}

struct Limiter {
    max_memory: Option<usize>,
    hit: bool,
}

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        if self.max_memory.is_some_and(|m| desired > m) {
            self.hit = true;
            return Ok(false);
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        Ok(desired <= 1_000_000)
    }

    fn instances(&self) -> usize {
        1
    }
    fn tables(&self) -> usize {
        16
    }
    fn memories(&self) -> usize {
        1
    }
}

struct State {
    limiter: Limiter,
    log: LogSink,
    host: HostCalls,
}

impl WasmModule for WiModule {
    fn instantiate(&self, limits: &WasmLimits, imports: &Imports) -> Result<Box<dyn WasmInstance>, WasmError> {
        let mut store = Store::new(
            &self.engine,
            State {
                limiter: Limiter { max_memory: limits.max_memory_bytes, hit: false },
                log: Arc::clone(&imports.log),
                host: HostCalls::new(imports.host.clone()),
            },
        );
        store.limiter(|s| &mut s.limiter);
        let mut linker = Linker::<State>::new(&self.engine);
        linker
            .func_wrap(IMPORT_MODULE, IMPORT_LOG, |caller: Caller<'_, State>, level: i32, ptr: i32, len: i32| {
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
        let mut inst =
            WiInstance { store, instance: None, memory: None, limits: limits.clone(), deadline: None, reserve: 0 };
        // The start function (if any) runs on the whole budget, without slicing.
        inst.store.set_fuel(limits.fuel.unwrap_or(u64::MAX)).map_err(|e| WasmError::Instantiate(e.to_string()))?;
        let instance = linker.instantiate_and_start(&mut inst.store, &self.module).map_err(|e| inst.map_err(e))?;
        let memory = instance
            .get_memory(&inst.store, EXPORT_MEMORY)
            .ok_or_else(|| WasmError::MissingExport(EXPORT_MEMORY.into()))?;
        inst.instance = Some(instance);
        inst.memory = Some(memory);
        Ok(Box::new(inst))
    }
}

struct WiInstance {
    store: Store<State>,
    instance: Option<Instance>,
    memory: Option<Memory>,
    limits: WasmLimits,
    /// Wall-clock deadline of the current call sequence.
    deadline: Option<Instant>,
    /// Fuel of the current budget not yet put into the store.
    reserve: u64,
}

impl WiInstance {
    fn map_err(&mut self, e: wasmi::Error) -> WasmError {
        if std::mem::take(&mut self.store.data_mut().limiter.hit) {
            return WasmError::MemoryLimit;
        }
        match e.as_trap_code() {
            Some(TrapCode::OutOfFuel) => WasmError::OutOfFuel,
            _ => WasmError::Trap(e.to_string()),
        }
    }

    fn memory(&self) -> Result<Memory, WasmError> {
        self.memory.ok_or_else(|| WasmError::MissingExport(EXPORT_MEMORY.into()))
    }
}

impl WasmInstance for WiInstance {
    fn begin_call(&mut self) -> Result<(), WasmError> {
        let budget = self.limits.fuel.unwrap_or(u64::MAX);
        self.deadline = self.limits.timeout.map(|t| Instant::now() + t);
        let grant = if self.deadline.is_some() { budget.min(FUEL_SLICE) } else { budget };
        self.reserve = budget - grant;
        self.store.set_fuel(grant).map_err(|e| WasmError::Instantiate(e.to_string()))
    }

    fn call(&mut self, func: &str, args: &[WasmVal]) -> Result<Option<WasmVal>, WasmError> {
        let instance = self.instance.ok_or_else(|| WasmError::MissingExport(func.into()))?;
        let f = instance.get_func(&self.store, func).ok_or_else(|| WasmError::MissingExport(func.into()))?;
        let params = to_engine_vals(args);
        let n = f.ty(&self.store).results().len();
        let mut results = vec![Val::I32(0); n];
        let mut state = f.call_resumable(&mut self.store, &params, &mut results);
        loop {
            match state {
                Ok(ResumableCall::Finished) => break,
                Ok(ResumableCall::HostTrap(t)) => return Err(WasmError::Trap(format!("host function failed: {t:?}"))),
                Ok(ResumableCall::OutOfFuel(pending)) => {
                    let need = pending.required_fuel();
                    if self.reserve == 0 || self.reserve < need {
                        return Err(WasmError::OutOfFuel);
                    }
                    if self.deadline.is_some_and(|d| Instant::now() >= d) {
                        return Err(WasmError::Timeout);
                    }
                    let grant = self.reserve.min(FUEL_SLICE.max(need));
                    self.reserve -= grant;
                    let left = self.store.get_fuel().unwrap_or(0);
                    self.store
                        .set_fuel(left.saturating_add(grant))
                        .map_err(|e| WasmError::Trap(format!("cannot refuel: {e}")))?;
                    state = pending.resume(&mut self.store, &mut results);
                }
                Err(e) => return Err(self.map_err(e)),
            }
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

    /// `(module (memory (export "memory") 1))`.
    const ONE_PAGE: &[u8] = b"\0asm\x01\0\0\0\x05\x03\x01\x00\x01\x07\x0a\x01\x06memory\x02\x00";

    #[test]
    fn guest_memory_access_is_bounds_checked_before_allocating() {
        let module = WasmiEngine::new().compile(ONE_PAGE).unwrap();
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
}
