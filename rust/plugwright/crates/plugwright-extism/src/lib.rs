//! extism backend for plugwright. A module built with an extism PDK runs as
//! a [`plugwright_core::Plugin`] under a memory cap and a per-call budget,
//! with the host functions the application's [`HostEnv`] grants, and on a
//! [`Worker`] of its own: control calls before events, and a call cancelled
//! through extism's `CancelHandle` at its deadline.
//!
//! plugwright's own Wasm ABI (`plugwright-wasm`) gives guests no WASI, a log
//! import and the named host functions the application registers (design
//! D13). extism guests speak extism's ABI instead, import the application's
//! host functions extism-style, and get WASI preview 1 only when the
//! application passes [`Wasi`] (design D27).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use extism::{CancelHandle, Function, Manifest, PluginBuilder, Wasm};
use plugwright_core::{Backend, Interrupt, Plugin, PluginError, PluginInfo, Result, WasmError, Worker};

/// Re-exported so host functions are built against the extism this crate runs.
pub use extism;
pub use plugwright_core::{CONTROL_DEPTH, EVENT_DEPTH, Lane};

const PAGES_PER_MIB: u32 = 16;

/// The application's side of one plugin: its host functions, and a hook told
/// which export is running so those functions can refuse what it may not do.
pub trait HostEnv: Send + Sync + 'static {
    /// The plugin's id; names its threads and its [`PluginInfo`].
    fn id(&self) -> &str;

    /// The host functions the module may import. Called once, at load.
    fn functions(self: &Arc<Self>) -> Vec<Function>;

    /// Called with the export about to run, and with `""` once it returns.
    fn enter(&self, _export: &str) {}
}

/// What the application fixes for every plugin it loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The one export a module must have; loading fails without it.
    pub required_export: &'static str,
    /// Thread names are `<prefix>-<id>` and `<prefix>-<id>-deadline`.
    pub thread_prefix: &'static str,
    /// Linear memory when the plugin asks for none, in MiB.
    pub memory_mib: u32,
    /// The most linear memory a plugin may ask for, in MiB.
    pub max_memory_mib: u32,
    /// Per-call budget when the plugin asks for none, in milliseconds.
    pub call_ms: u32,
}

impl Options {
    /// Options with the default limits: 16 MiB of memory (at most 64) and a
    /// 200 ms call budget.
    pub const fn new(required_export: &'static str, thread_prefix: &'static str) -> Self {
        Self { required_export, thread_prefix, memory_mib: 16, max_memory_mib: 64, call_ms: 200 }
    }
}

/// What one plugin asks for. Absent means the [`Options`] default; memory is
/// clamped to [`Options::max_memory_mib`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    /// Linear memory in MiB.
    pub memory_mib: Option<u32>,
    /// Per-call budget in milliseconds; also the cap on every call's deadline.
    pub call_ms: Option<u32>,
}

/// WASI preview 1 for one plugin (wasip1 guests: Go, Kotlin). The guest sees
/// no environment variables, no arguments, and only these directories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wasi {
    /// `(host, guest)` directory pairs. A host path prefixed with `ro:` is
    /// mounted read-only (extism's convention).
    pub paths: Vec<(String, PathBuf)>,
}

/// An extism module as a [`Plugin`]. [`load`] puts one on a [`Worker`]; use
/// this type directly to run it on a thread of your own.
pub struct ExtismPlugin<E: HostEnv> {
    plugin: extism::Plugin,
    cancel: CancelHandle,
    info: PluginInfo,
    env: Arc<E>,
    call_cap: Duration,
}

impl<E: HostEnv> ExtismPlugin<E> {
    /// Compiles `wasm` (binary or WAT text) with no compilation cache, under
    /// `limits` clamped by `options`, with `env`'s host functions, and with
    /// WASI only when `wasi` is given.
    ///
    /// Each plugin gets its own wasmtime `Engine`, because extism builds one
    /// inside `CompiledPlugin::new` and takes no shared one; nothing here
    /// moves a wasmtime object from one plugin to another.
    pub fn new(wasm: &[u8], limits: &Limits, options: &Options, env: Arc<E>, wasi: Option<&Wasi>) -> Result<Self> {
        let mib = limits.memory_mib.unwrap_or(options.memory_mib).min(options.max_memory_mib);
        let call_cap = Duration::from_millis(limits.call_ms.unwrap_or(options.call_ms).into());
        let mut manifest = Manifest::new([Wasm::data(wasm.to_vec())])
            .with_memory_max(mib.saturating_mul(PAGES_PER_MIB))
            .with_timeout(call_cap);
        for (host, guest) in wasi.map(|w| w.paths.as_slice()).unwrap_or_default() {
            manifest = manifest.with_allowed_path(host.clone(), guest);
        }
        // The default cache writes under the user's system cache dir, which
        // belongs to no application in particular.
        let builder = PluginBuilder::new(manifest)
            .with_wasi(wasi.is_some())
            .with_cache_disabled()
            .with_functions(env.functions());
        #[cfg(feature = "pulley")]
        let builder = builder.with_wasmtime_config(pulley_config()?);
        let plugin = builder.build().map_err(|e| PluginError::Load(format!("{e:#}")))?;
        if !plugin.function_exists(options.required_export) {
            return Err(WasmError::MissingExport(options.required_export.to_owned()).into());
        }
        let info = PluginInfo {
            name: env.id().to_owned(),
            // An extism module carries no version; the application's own
            // package manifest does.
            version: semver::Version::new(0, 0, 0),
            // Built against an extism PDK, not plugwright-sdk: there is no SDK
            // to check, so report the host's and let `check_sdk` pass.
            sdk_version: plugwright_core::host_sdk_version(),
            backend: Backend::Wasm,
            engine: Some("extism".to_owned()),
            sandbox: None,
        };
        Ok(Self { cancel: plugin.cancel_handle(), plugin, info, env, call_cap })
    }

    /// The plugin's per-call budget: [`Limits::call_ms`] or the default.
    pub fn call_cap(&self) -> Duration {
        self.call_cap
    }
}

impl<E: HostEnv> Plugin for ExtismPlugin<E> {
    fn info(&self) -> &PluginInfo {
        &self.info
    }

    fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        if !self.plugin.function_exists(method) {
            return Err(WasmError::MissingExport(method.to_owned()).into());
        }
        self.env.enter(method);
        let out = self.plugin.call::<&[u8], Vec<u8>>(method, input);
        self.env.enter("");
        out.map_err(|e| call_error(&e, self.call_cap))
    }

    fn has_method(&self, method: &str) -> bool {
        self.plugin.function_exists(method)
    }

    fn interrupter(&self) -> Option<Interrupt> {
        let cancel = self.cancel.clone();
        // extism's timer ignores a cancel when no call is running.
        Some(Arc::new(move || {
            let _ = cancel.cancel();
        }))
    }
}

/// Loads `wasm` without WASI and starts its worker, named
/// `<thread_prefix>-<id>`. Call it with [`Worker::call_json`].
pub fn load<E: HostEnv>(wasm: &[u8], limits: &Limits, options: &Options, env: Arc<E>) -> Result<Worker> {
    spawn(ExtismPlugin::new(wasm, limits, options, env, None)?, options)
}

/// [`load`] with WASI preview 1 over `wasi`'s directories.
pub fn load_with_wasi<E: HostEnv>(
    wasm: &[u8],
    limits: &Limits,
    options: &Options,
    env: Arc<E>,
    wasi: &Wasi,
) -> Result<Worker> {
    spawn(ExtismPlugin::new(wasm, limits, options, env, Some(wasi))?, options)
}

fn spawn<E: HostEnv>(plugin: ExtismPlugin<E>, options: &Options) -> Result<Worker> {
    let name = format!("{}-{}", options.thread_prefix, plugin.env.id());
    let call_cap = plugin.call_cap;
    Worker::spawn(Box::new(plugin), &name, call_cap)
}

/// Maps a failed call. extism reports its own two limits as bare messages:
/// `"timeout"` for an epoch interrupt (a `CancelHandle` or the manifest
/// `timeout_ms`) and `"oom"` from its memory limiter.
fn call_error(err: &extism::Error, call_cap: Duration) -> PluginError {
    match err.root_cause().to_string().as_str() {
        "timeout" => PluginError::Timeout(call_cap),
        "oom" => WasmError::MemoryLimit.into(),
        message => WasmError::Trap(message.to_owned()).into(),
    }
}

/// The Pulley target of this host's pointer width. iOS and Android are
/// little-endian, so the big-endian variants are not needed.
#[cfg(feature = "pulley")]
const PULLEY: &str = if cfg!(target_pointer_width = "64") { "pulley64" } else { "pulley32" };

/// Cranelift emits Pulley bytecode instead of machine code, so loading a
/// plugin never maps executable memory, which iOS forbids. extism keeps the
/// target and only overrides the proposals and interruption it needs.
#[cfg(feature = "pulley")]
fn pulley_config() -> Result<wasmtime::Config> {
    let mut config = wasmtime::Config::new();
    config.target(PULLEY).map_err(|e| PluginError::Load(format!("{e:#}")))?;
    Ok(config)
}

#[cfg(test)]
mod tests;
