//! Compile and load plugins with nothing but the checks that keep the host
//! sound, for hosts that do not need a manifest, a lockfile or process
//! isolation.
//!
//! * [`NativeLoader`]: shared and static libraries mapped into this process,
//!   as [`NativePlugin`].
//! * `WasmLoader` (feature `wasm`, implied by `wasmtime` and `wasmi`): core
//!   Wasm modules compiled once and run on an engine, as `WasmPlugin`.
//! * [`Loader`]: both over one cache; [`open`](Loader::open) picks by the
//!   file's magic bytes.
//!
//! ```no_run
//! # fn main() -> plugwright_core::Result<()> {
//! use plugwright_loader::Loader;
//!
//! let loader = Loader::new(".plugwright")?;
//! let dynamic = loader.open("plugins/libhello.so")?; // mapped in place
//! let stat = loader.open("plugins/libhello.a")?; // relinked once, cached
//! let wasm = loader.open("plugins/hello.wasm")?; // compiled once, cached
//! let mut built = loader.load("plugins/hello")?; // cargo build, cached
//! let out = built.call("greet", b"world")?;
//! # let _ = (dynamic, stat, wasm, out);
//! # Ok(())
//! # }
//! ```
//!
//! The `plugwright` crate builds its manifest-driven `PluginManager` on top
//! of this one and re-exports it.

use std::path::{Path, PathBuf};

use plugwright_build::builder::{BuildOptions, BuildRequest, Builder, Target};
use plugwright_build::cache::{Artifact, Cache};
use plugwright_core::{Plugin, Result, io_err};
use plugwright_native::format::{self, Format};

mod native;
#[cfg(feature = "wasm")]
mod wasm;

pub use native::NativeLoader;
pub use plugwright_native::NativePlugin;
#[cfg(feature = "wasm")]
pub use plugwright_wasm::{WasmEngine, WasmLimits, WasmPlugin};
#[cfg(feature = "wasmi")]
pub use plugwright_wasmi::WasmiEngine;
#[cfg(feature = "wasmtime")]
pub use plugwright_wasmtime::WasmtimeEngine;
#[cfg(feature = "wasm")]
pub use wasm::{WasmLoader, builtin_engines};

/// [`NativeLoader`] and `WasmLoader` over one cache: opens any plugin file,
/// builds a crate for either target.
pub struct Loader {
    native: NativeLoader,
    #[cfg(feature = "wasm")]
    wasm: WasmLoader,
}

impl Loader {
    /// Release builds; built, relinked and compiled plugins are cached in `cache_dir`.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        Self::with_options(cache_dir, BuildOptions::default())
    }

    pub fn with_options(cache_dir: impl Into<PathBuf>, opts: BuildOptions) -> Result<Self> {
        let cache = Cache::new(cache_dir.into())?;
        Ok(Loader {
            #[cfg(feature = "wasm")]
            wasm: WasmLoader::with_builder(Builder::new(cache.clone(), opts.clone())),
            native: NativeLoader::with_builder(Builder::new(cache, opts)),
        })
    }

    /// Run Wasm plugins on `engine` (see [`WasmLoader::with_engine`]).
    #[cfg(feature = "wasm")]
    pub fn with_engine(mut self, engine: std::sync::Arc<dyn WasmEngine>) -> Self {
        self.wasm = self.wasm.with_engine(engine);
        self
    }

    /// Limits for Wasm plugins loaded from now on (see [`WasmLoader::with_limits`]).
    #[cfg(feature = "wasm")]
    pub fn with_limits(mut self, limits: WasmLimits) -> Self {
        self.wasm = self.wasm.with_limits(limits);
        self
    }

    pub fn native(&self) -> &NativeLoader {
        &self.native
    }

    #[cfg(feature = "wasm")]
    pub fn wasm(&self) -> &WasmLoader {
        &self.wasm
    }

    pub fn cache(&self) -> &Cache {
        self.native.cache()
    }

    /// Open a shared library in place, relink a static one once, or compile a
    /// Wasm module, by the file's magic bytes. The plugin's `new` gets `{}` as
    /// its configuration.
    pub fn open(&self, path: impl AsRef<Path>) -> Result<Box<dyn Plugin>> {
        self.open_with(path, b"{}")
    }

    /// [`open`](Self::open) with `config` passed verbatim to the plugin's `new`.
    pub fn open_with(&self, path: impl AsRef<Path>, config: &[u8]) -> Result<Box<dyn Plugin>> {
        let path = path.as_ref();
        let id = id_for(path);
        match format::sniff(path)? {
            #[cfg(feature = "wasm")]
            Format::Wasm | Format::WasmComponent => Ok(Box::new(self.wasm.open_as(&id, path, config)?)),
            #[cfg(not(feature = "wasm"))]
            f @ (Format::Wasm | Format::WasmComponent) => Err(plugwright_core::PluginError::Unsupported(format!(
                "{}: {f}; this build has no Wasm support (feature `wasm`)",
                path.display()
            ))),
            f => Ok(Box::new(self.native.open_as(&id, path, &f, config)?)),
        }
    }

    /// [`NativeLoader::build`].
    pub fn build(&self, crate_dir: impl AsRef<Path>) -> Result<Artifact> {
        self.native.build(crate_dir)
    }

    /// [`NativeLoader::load`].
    pub fn load(&self, crate_dir: impl AsRef<Path>) -> Result<Box<dyn Plugin>> {
        Ok(Box::new(self.native.load(crate_dir)?))
    }

    /// [`NativeLoader::load_with`].
    pub fn load_with(&self, crate_dir: impl AsRef<Path>, config: &[u8]) -> Result<Box<dyn Plugin>> {
        Ok(Box::new(self.native.load_with(crate_dir, config)?))
    }

    /// [`WasmLoader::build`].
    #[cfg(feature = "wasm")]
    pub fn build_wasm(&self, crate_dir: impl AsRef<Path>) -> Result<Artifact> {
        self.wasm.build(crate_dir)
    }

    /// [`WasmLoader::load`].
    #[cfg(feature = "wasm")]
    pub fn load_wasm(&self, crate_dir: impl AsRef<Path>) -> Result<Box<dyn Plugin>> {
        Ok(Box::new(self.wasm.load(crate_dir)?))
    }

    /// [`WasmLoader::load_with`].
    #[cfg(feature = "wasm")]
    pub fn load_wasm_with(&self, crate_dir: impl AsRef<Path>, config: &[u8]) -> Result<Box<dyn Plugin>> {
        Ok(Box::new(self.wasm.load_with(crate_dir, config)?))
    }
}

/// `cargo build` of the crate in `crate_dir` for `target`, through the cache.
fn cargo_build(builder: &Builder, crate_dir: &Path, target: Target) -> Result<(String, Artifact)> {
    let crate_dir = std::path::absolute(crate_dir).map_err(io_err(crate_dir))?;
    let id = id_for(&crate_dir);
    let built = builder.build(&BuildRequest {
        id: id.clone(),
        source_label: format!("path+{}", crate_dir.display()),
        crate_dir,
        target,
        features: Vec::new(),
        no_default_features: false,
        in_process: target == Target::Native,
    })?;
    Ok((id, built))
}

/// A log and error label from a path: `plugins/libhello.so.1` → `hello`.
fn id_for(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let stem = name.split('.').next().unwrap_or_default();
    let stem = stem.strip_prefix("lib").filter(|s| !s.is_empty()).unwrap_or(stem);
    let id: String = stem.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
    if id.is_empty() { "plugin".to_owned() } else { id }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugwright_core::PluginError;

    #[test]
    fn ids_come_from_the_file_or_crate_name() {
        for (path, id) in [
            ("plugins/libhello.so.1", "hello"),
            ("hello.dll", "hello"),
            ("C:/x/libhello.dylib", "hello"),
            ("libhello.a", "hello"),
            ("hello.wasm", "hello"),
            ("lib.so", "lib"),
            ("plugins/my-crate", "my-crate"),
            ("plugins/ünï code", "ncode"),
            ("/", "plugin"),
        ] {
            assert_eq!(id_for(Path::new(path)), id, "{path}");
        }
        assert_eq!(id_for(Path::new(&"a".repeat(100))).len(), 64);
    }

    #[test]
    fn files_go_to_the_loader_their_magic_bytes_name() {
        let dir = tempfile::tempdir().unwrap();
        let loader = Loader::new(dir.path().join("cache")).unwrap();
        let open = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            loader.open(&path).map(|p| p.info().name.clone())
        };
        match open("notes.so", b"just text") {
            Err(PluginError::NotAPlugin { reason, .. }) => assert!(reason.contains("not a shared library"), "{reason}"),
            other => panic!("{other:?}"),
        }
        // Wasm, whatever the file name: never handed to the native loader.
        let wasm = open("module.dll", b"\0asm\x01\0\0\0");
        #[cfg(feature = "wasm")]
        assert!(matches!(wasm, Err(PluginError::Wasm(_) | PluginError::Unsupported(_))), "{wasm:?}");
        #[cfg(not(feature = "wasm"))]
        assert!(matches!(&wasm, Err(PluginError::Unsupported(e)) if e.contains("feature `wasm`")), "{wasm:?}");
        assert!(matches!(open("c.wasm", b"\0asm\x0d\0\x01\0"), Err(PluginError::Unsupported(_))));
        assert!(matches!(loader.open(dir.path().join("missing.so")), Err(PluginError::Io { .. })));
    }
}
