//! [`NativeLoader`]: shared and static libraries, mapped into this process.

use std::path::{Path, PathBuf};

use plugwright_build::builder::{BuildOptions, Builder, Target};
use plugwright_build::cache::{Artifact, Cache, sha256_file};
use plugwright_build::relink::RelinkRequest;
use plugwright_core::{Result, tracing_sink};
use plugwright_native::format::{self, Format};
use plugwright_native::{NativePlugin, Placement};

use crate::{cargo_build, id_for};

/// Builds plugin crates as native libraries and maps them into this process.
///
/// What runs on every load: the file format from its magic bytes, the ABI
/// version and declaration table, the SDK version, the plugin's own `new`.
/// What does not: hashing a shared library, copying it, a lockfile, a runner
/// process, a sandbox. Libraries are mapped in place, so a file must not be
/// rewritten while it is loaded; a second load of the same file gets a private
/// copy (each instance needs an image of its own).
///
/// The library's code runs unconfined in this process: use it for code you
/// trust, and `plugwright::PluginManager` with process isolation for code you
/// do not.
pub struct NativeLoader {
    builder: Builder,
}

impl NativeLoader {
    /// Release builds; built and relinked libraries are cached in `cache_dir`.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        Self::with_options(cache_dir, BuildOptions::default())
    }

    pub fn with_options(cache_dir: impl Into<PathBuf>, opts: BuildOptions) -> Result<Self> {
        Ok(Self::with_builder(Builder::new(Cache::new(cache_dir.into())?, opts)))
    }

    pub(crate) fn with_builder(builder: Builder) -> Self {
        NativeLoader { builder }
    }

    pub fn cache(&self) -> &Cache {
        self.builder.cache()
    }

    /// Open a shared library (`.so`, `.dylib`, `.dll`, any file name) in place,
    /// or relink a static one (`.a`, `.lib`) once and open the cached result.
    /// The plugin's `new` gets `{}` as its configuration.
    pub fn open(&self, path: impl AsRef<Path>) -> Result<NativePlugin> {
        self.open_with(path, b"{}")
    }

    /// [`open`](Self::open) with `config` passed verbatim to the plugin's `new`.
    pub fn open_with(&self, path: impl AsRef<Path>, config: &[u8]) -> Result<NativePlugin> {
        let path = path.as_ref();
        self.open_as(&id_for(path), path, &format::sniff(path)?, config)
    }

    /// Build the plugin crate in `crate_dir` (a `cdylib`, or a `staticlib`
    /// relinked into one), or return the cached build for unchanged inputs.
    pub fn build(&self, crate_dir: impl AsRef<Path>) -> Result<Artifact> {
        let (id, built) = cargo_build(&self.builder, crate_dir.as_ref(), Target::Native)?;
        match format::sniff(&built.path)? {
            // The cache already knows the archive's hash.
            Format::Archive => self.relink(&id, &built.path, built.sha256.clone()),
            _ => Ok(built),
        }
    }

    /// [`build`](Self::build), then open the library in place.
    pub fn load(&self, crate_dir: impl AsRef<Path>) -> Result<NativePlugin> {
        self.load_with(crate_dir, b"{}")
    }

    /// [`load`](Self::load) with `config` passed verbatim to the plugin's `new`.
    pub fn load_with(&self, crate_dir: impl AsRef<Path>, config: &[u8]) -> Result<NativePlugin> {
        let artifact = self.build(&crate_dir)?;
        self.open_in_place(&id_for(crate_dir.as_ref()), &artifact.path, config)
    }

    /// Open `path`, already sniffed as `format`.
    pub(crate) fn open_as(&self, id: &str, path: &Path, format: &Format, config: &[u8]) -> Result<NativePlugin> {
        match format {
            Format::Archive => {
                let library = self.relink(id, path, sha256_file(path)?)?.path;
                self.open_in_place(id, &library, config)
            }
            // Anything else that is not a loadable library is refused there.
            _ => self.open_in_place(id, path, config),
        }
    }

    fn open_in_place(&self, id: &str, library: &Path, config: &[u8]) -> Result<NativePlugin> {
        let copies = self.cache().shadow_dir();
        NativePlugin::load(id, library, Placement::InPlace { copies: Some(&copies) }, config, tracing_sink(id))
    }

    fn relink(&self, id: &str, archive: &Path, archive_sha256: String) -> Result<Artifact> {
        self.builder.relink(&RelinkRequest {
            id: id.to_owned(),
            archive: archive.to_owned(),
            archive_sha256,
            link_args: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugwright_core::{Plugin, PluginError};

    #[test]
    fn what_is_not_a_native_library_is_refused_before_loading() {
        let dir = tempfile::tempdir().unwrap();
        let loader = NativeLoader::new(dir.path().join("cache")).unwrap();
        for (name, bytes, reason) in
            [("notes.so", &b"just text"[..], "not a shared library"), ("module.dll", b"\0asm\x01\0\0\0", "Wasm module")]
        {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            match loader.open(&path) {
                Err(PluginError::NotAPlugin { reason: r, .. }) => assert!(r.contains(reason), "{r}"),
                other => panic!("{name}: {:?}", other.map(|p| p.info().name.clone())),
            }
        }
        assert!(matches!(loader.open(dir.path().join("missing.so")), Err(PluginError::Io { .. })));
        assert!(matches!(loader.build(dir.path().join("no-crate")), Err(PluginError::Build { .. })));
    }
}
