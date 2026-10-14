//! [`WasmLoader`]: Wasm modules on a pluggable engine, compiled once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use plugwright_build::builder::{BuildOptions, Builder, Target};
use plugwright_build::cache::{Artifact, Cache};
use plugwright_core::{PluginError, Result, io_err, tracing_sink};
use plugwright_native::format::{self, Format};
use plugwright_wasm::{ModuleCache, WasmEngine, WasmLimits, WasmPlugin};

use crate::{cargo_build, id_for};

/// Builds plugin crates for `wasm32-unknown-unknown` and runs them on a
/// [`WasmEngine`] (by default the first of [`builtin_engines`]).
///
/// Modules are compiled once per engine and content: later loads in this
/// process share the compiled module, and engines that can (wasmtime) keep the
/// machine code under the cache's `compiled/` for the next process. That code
/// is checked against its sha256 before use, which catches damage, not
/// tampering: anyone who can write to the cache directory can run code in
/// this process.
pub struct WasmLoader {
    builder: Builder,
    /// Created on the first load, so constructing a loader costs nothing.
    engine: OnceLock<Option<Arc<dyn WasmEngine>>>,
    modules: ModuleCache,
    limits: WasmLimits,
}

impl WasmLoader {
    /// Release builds; built modules and compiled code are cached in `cache_dir`.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        Self::with_options(cache_dir, BuildOptions::default())
    }

    pub fn with_options(cache_dir: impl Into<PathBuf>, opts: BuildOptions) -> Result<Self> {
        Ok(Self::with_builder(Builder::new(Cache::new(cache_dir.into())?, opts)))
    }

    pub(crate) fn with_builder(builder: Builder) -> Self {
        WasmLoader {
            modules: ModuleCache::new(Some(builder.cache().compiled_dir())),
            builder,
            engine: OnceLock::new(),
            limits: WasmLimits::default(),
        }
    }

    /// Run plugins on `engine` instead of the first of [`builtin_engines`].
    pub fn with_engine(mut self, engine: Arc<dyn WasmEngine>) -> Self {
        self.engine = OnceLock::from(Some(engine));
        self
    }

    /// Fuel, memory and time limits for plugins loaded from now on.
    pub fn with_limits(mut self, limits: WasmLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn cache(&self) -> &Cache {
        self.builder.cache()
    }

    /// The engine plugins run on, if this build has one.
    pub fn engine(&self) -> Option<&Arc<dyn WasmEngine>> {
        self.engine.get_or_init(default_engine).as_ref()
    }

    /// Compile (or reuse) and instantiate the core module at `path`, whatever
    /// its file name. The plugin's `new` gets `{}` as its configuration.
    pub fn open(&self, path: impl AsRef<Path>) -> Result<WasmPlugin> {
        self.open_with(path, b"{}")
    }

    /// [`open`](Self::open) with `config` passed verbatim to the plugin's `new`.
    pub fn open_with(&self, path: impl AsRef<Path>, config: &[u8]) -> Result<WasmPlugin> {
        let path = path.as_ref();
        self.open_as(&id_for(path), path, config)
    }

    /// Build the plugin crate in `crate_dir` for `wasm32-unknown-unknown`, or
    /// return the cached build for unchanged inputs.
    pub fn build(&self, crate_dir: impl AsRef<Path>) -> Result<Artifact> {
        Ok(cargo_build(&self.builder, crate_dir.as_ref(), Target::Wasm)?.1)
    }

    /// [`build`](Self::build), then compile and instantiate the module.
    pub fn load(&self, crate_dir: impl AsRef<Path>) -> Result<WasmPlugin> {
        self.load_with(crate_dir, b"{}")
    }

    /// [`load`](Self::load) with `config` passed verbatim to the plugin's `new`.
    pub fn load_with(&self, crate_dir: impl AsRef<Path>, config: &[u8]) -> Result<WasmPlugin> {
        let artifact = self.build(&crate_dir)?;
        self.open_as(&id_for(crate_dir.as_ref()), &artifact.path, config)
    }

    pub(crate) fn open_as(&self, id: &str, path: &Path, config: &[u8]) -> Result<WasmPlugin> {
        let wasm = std::fs::read(path).map_err(io_err(path))?;
        match format::detect(&wasm) {
            Format::Wasm => {}
            Format::WasmComponent => {
                return Err(PluginError::Unsupported(format!(
                    "{}: wasm components; build a core module (wasm32-unknown-unknown)",
                    path.display()
                )));
            }
            other => {
                return Err(PluginError::NotAPlugin {
                    path: path.to_owned(),
                    reason: format!("{other}, not a Wasm module"),
                });
            }
        }
        let engine = self.engine().ok_or_else(|| {
            PluginError::Unsupported("no wasm engine: enable the `wasmtime` or `wasmi` feature".into())
        })?;
        let module = self.modules.compile(engine, &wasm)?;
        WasmPlugin::with_module(id, engine.name(), module, self.limits.clone(), config, tracing_sink(id))
    }
}

/// Engines compiled into this build (features `wasmtime`, `wasmi`), by name.
#[allow(clippy::vec_init_then_push)] // the pushes depend on enabled features
pub fn builtin_engines() -> Vec<Arc<dyn WasmEngine>> {
    #[allow(unused_mut)]
    let mut v: Vec<Arc<dyn WasmEngine>> = Vec::new();
    #[cfg(feature = "wasmtime")]
    match plugwright_wasmtime::WasmtimeEngine::new() {
        Ok(e) => v.push(Arc::new(e)),
        Err(e) => tracing::error!(error = %e, "wasmtime engine unavailable"),
    }
    #[cfg(feature = "wasmi")]
    v.push(Arc::new(plugwright_wasmi::WasmiEngine::new()));
    v
}

/// The first of [`builtin_engines`], without creating the others.
fn default_engine() -> Option<Arc<dyn WasmEngine>> {
    #[cfg(feature = "wasmtime")]
    match plugwright_wasmtime::WasmtimeEngine::new() {
        Ok(e) => return Some(Arc::new(e)),
        Err(e) => tracing::error!(error = %e, "wasmtime engine unavailable"),
    }
    #[cfg(feature = "wasmi")]
    return Some(Arc::new(plugwright_wasmi::WasmiEngine::new()));
    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_not_a_core_module_is_refused_before_compiling() {
        let dir = tempfile::tempdir().unwrap();
        let loader = WasmLoader::new(dir.path().join("cache")).unwrap();
        let open = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            loader.open(&path).map(|_| ())
        };
        match open("libhello.so", b"\x7fELF\x02\x01\x01\0") {
            Err(PluginError::NotAPlugin { reason, .. }) => assert!(reason.contains("not a Wasm module"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(open("c.wasm", b"\0asm\x0d\0\x01\0"), Err(PluginError::Unsupported(_))));
        assert!(matches!(loader.open(dir.path().join("missing.wasm")), Err(PluginError::Io { .. })));
        // A valid but empty core module, whatever its name: the engine gets it
        // and finds no plugin exports.
        match (open("module.bin", b"\0asm\x01\0\0\0"), loader.engine()) {
            (Err(PluginError::Wasm(e)), Some(_)) => assert!(e.to_string().contains("export"), "{e}"),
            (Err(PluginError::Unsupported(e)), None) => assert!(e.contains("no wasm engine"), "{e}"),
            (other, _) => panic!("{other:?}"),
        }
    }

    #[test]
    #[cfg(any(feature = "wasmtime", feature = "wasmi"))]
    fn the_engine_is_chosen_once_and_can_be_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let loader = WasmLoader::new(dir.path()).unwrap();
        let first = builtin_engines().first().map(|e| e.name().to_owned());
        assert_eq!(loader.engine().map(|e| e.name().to_owned()), first);
        assert!(Arc::ptr_eq(loader.engine().unwrap(), loader.engine().unwrap()));
        let last = builtin_engines().pop().unwrap();
        let loader = loader.with_engine(Arc::clone(&last));
        assert!(Arc::ptr_eq(loader.engine().unwrap(), &last));
    }
}
