//! [`PluginManager`]: resolve → build/cache → load → call → unload.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::builder::{BuildOptions, BuildRequest, Builder, Target};
use crate::cache::{Artifact, Cache, sha256_file, verify_sha256};
use crate::client::{Bound, MethodInfo};
use crate::error::{PluginError, Result, io_err};
use crate::git;
use crate::isolate::{IsolatedPlugin, IsolationOptions, RunnerCommand};
use crate::loader::builtin_engines;
use crate::logging::tracing_sink;
use crate::manifest::{Isolation, LockEntry, Lockfile, Manifest, PluginKind, PluginSpec, Source};
use crate::native::format::{self, Format};
use crate::native::{NativePlugin, Placement};
use crate::plugin::{Plugin, PluginInfo, Release};
use crate::relink::RelinkRequest;
use crate::wasm::{ModuleCache, WasmEngine, WasmPlugin};
use crate::watch::{Watch, WatchOptions, Watcher};

/// Manager-wide configuration.
#[derive(Debug, Clone)]
pub struct ManagerOptions {
    pub cache_dir: PathBuf,
    /// Relative plugin paths resolve against this directory.
    pub base_dir: PathBuf,
    pub build: BuildOptions,
    pub isolation: IsolationOptions,
    pub allow_local_git: bool,
    /// Where to persist resolved SHAs and hashes; `None` keeps the lock in memory.
    pub lock_path: Option<PathBuf>,
}

impl ManagerOptions {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        ManagerOptions {
            cache_dir: base_dir.join(".plugwright"),
            base_dir,
            build: BuildOptions::default(),
            isolation: IsolationOptions::default(),
            allow_local_git: false,
            lock_path: None,
        }
    }

    /// Options derived from a manifest's `[settings]`.
    pub fn from_manifest(m: &Manifest) -> Self {
        let s = &m.settings;
        let mut o = ManagerOptions::new(&m.base_dir);
        o.cache_dir = m.resolve(&s.cache_dir);
        o.build.locked = s.locked;
        o.build.offline = s.offline;
        o.build.release = s.release;
        o.build.accel = s.build.clone();
        o.allow_local_git = s.allow_local_git;
        o.isolation.max_restarts = s.max_restarts;
        if let Some(r) = &s.runner {
            o.isolation.runner = RunnerCommand::Path(m.resolve(r));
        }
        o.lock_path = Some(m.lock_path());
        o
    }
}

struct Loaded {
    spec: PluginSpec,
    artifact: Artifact,
    plugin: Box<dyn Plugin>,
}

/// Owns every loaded plugin. Dropping the manager unloads them all.
pub struct PluginManager {
    opts: ManagerOptions,
    builder: Builder,
    engines: Vec<Arc<dyn WasmEngine>>,
    modules: ModuleCache,
    plugins: BTreeMap<String, Loaded>,
    lock: Lockfile,
    leaked: usize,
}

impl PluginManager {
    /// Create a manager with the Wasm engines compiled into this build.
    pub fn new(opts: ManagerOptions) -> Result<Self> {
        let cache = Cache::new(&opts.cache_dir)?;
        let lock = match &opts.lock_path {
            Some(p) => Lockfile::load(p)?,
            None => Lockfile { version: Lockfile::VERSION, plugins: vec![] },
        };
        Ok(PluginManager {
            modules: ModuleCache::new(Some(cache.compiled_dir())),
            builder: Builder::new(cache, opts.build.clone()),
            opts,
            engines: builtin_engines(),
            plugins: BTreeMap::new(),
            lock,
            leaked: 0,
        })
    }

    /// Read a manifest and create a manager configured by its `[settings]`.
    pub fn from_manifest(path: &Path) -> Result<(Self, Manifest)> {
        let m = Manifest::load(path)?;
        let mgr = Self::new(ManagerOptions::from_manifest(&m))?;
        Ok((mgr, m))
    }

    /// Add (or replace, by name) a Wasm engine.
    pub fn register_engine(&mut self, engine: Arc<dyn WasmEngine>) {
        self.engines.retain(|e| e.name() != engine.name());
        tracing::info!(engine = engine.name(), "wasm engine registered");
        self.engines.push(engine);
    }

    pub fn engines(&self) -> impl Iterator<Item = &str> {
        self.engines.iter().map(|e| e.name())
    }

    fn engine(&self, name: Option<&str>) -> Result<Arc<dyn WasmEngine>> {
        match name {
            Some(n) => self.engines.iter().find(|e| e.name() == n).cloned().ok_or_else(|| {
                PluginError::Unsupported(format!(
                    "wasm engine `{n}` is not registered (available: {})",
                    self.engines().collect::<Vec<_>>().join(", ")
                ))
            }),
            None => self
                .engines
                .first()
                .cloned()
                .ok_or_else(|| PluginError::Unsupported("no wasm engine registered".into())),
        }
    }

    pub fn cache(&self) -> &Cache {
        self.builder.cache()
    }

    fn resolve_path(&self, p: &Path) -> PathBuf {
        if p.is_absolute() { p.to_owned() } else { self.opts.base_dir.join(p) }
    }

    /// Resolve the source and build (or fetch from cache) without loading, and
    /// pin the result in the lockfile: an explicit build is the user choosing
    /// that artifact. `load`/`reload` pin only what actually loaded.
    pub fn prepare(&mut self, spec: &PluginSpec) -> Result<Artifact> {
        let (artifact, entry) = self.resolve(spec)?;
        self.pin(entry);
        Ok(artifact)
    }

    /// `prepare` without touching the lockfile.
    fn resolve(&self, spec: &PluginSpec) -> Result<(Artifact, LockEntry)> {
        spec.validate()?;
        let target = match spec.kind {
            PluginKind::Native => Target::Native,
            PluginKind::Wasm => Target::Wasm,
        };
        let locked = self.lock.get(&spec.id).cloned();
        let (artifact, label, git_sha) = match &spec.source {
            Source::Artifact(p) => {
                let path = self.resolve_path(p);
                let sha256 = sha256_file(&path)?;
                (Artifact { path, sha256, key: None, cached: true }, format!("artifact+{}", p.display()), None)
            }
            Source::Path(p) => {
                let dir = self.resolve_path(p);
                let label = format!("path+{}", p.display());
                (self.build(spec, target, dir, label.clone())?, label, None)
            }
            Source::Git(g) => {
                let label_matches = locked.as_ref().is_some_and(|l| l.source.starts_with(&g.label("")));
                let pinned = locked.as_ref().filter(|_| label_matches).and_then(|l| l.git_sha.as_deref());
                let (dir, sha) = git::checkout(
                    &self.cache().git_dir(),
                    g,
                    pinned,
                    self.opts.allow_local_git,
                    self.opts.build.offline,
                )?;
                let label = g.label(&sha);
                (self.build(spec, target, dir, label.clone())?, label, Some(sha))
            }
        };

        // The pin covers what the user supplied or cargo produced, i.e. the
        // static library itself when it is relinked below.
        if let Some(expected) = &spec.sha256 {
            verify_sha256(&artifact.path, expected)?;
        }
        let artifact = self.normalize(spec, artifact)?;
        // Same inputs must give the same bytes as last time; otherwise the
        // cache (artifact *and* its entry.json) was tampered with.
        if let Some(l) = &locked
            && artifact.key.is_some()
            && l.cache_key == artifact.key
            && l.sha256 != artifact.sha256
        {
            return Err(PluginError::Integrity {
                path: artifact.path.clone(),
                expected: l.sha256.clone(),
                actual: artifact.sha256.clone(),
            });
        }
        let entry = LockEntry {
            id: spec.id.clone(),
            source: label,
            git_sha,
            cache_key: artifact.key.clone(),
            sha256: artifact.sha256.clone(),
        };
        Ok((artifact, entry))
    }

    fn pin(&mut self, entry: LockEntry) {
        self.lock.upsert(entry);
        self.save_lock();
    }

    /// Check the artifact's real format (magic bytes, not the extension) and
    /// relink a native static library into a shared one.
    fn normalize(&self, spec: &PluginSpec, artifact: Artifact) -> Result<Artifact> {
        let format = format::sniff(&artifact.path)?;
        let wrong = |reason: String| Err(PluginError::NotAPlugin { path: artifact.path.clone(), reason });
        match (spec.kind, &format) {
            (PluginKind::Native, Format::Archive) => {
                tracing::info!(plugin = %spec.id, "static library; relinking into a shared library");
                self.builder.relink(&RelinkRequest {
                    id: spec.id.clone(),
                    archive: artifact.path.clone(),
                    archive_sha256: artifact.sha256.clone(),
                    link_args: spec.link_args.clone(),
                })
            }
            // Checked here too, so `isolation = "process"` fails before a runner is spawned.
            (PluginKind::Native, _) => format::check_loadable(&artifact.path).map(|_| artifact),
            (PluginKind::Wasm, Format::Wasm) => Ok(artifact),
            (PluginKind::Wasm, Format::WasmComponent) => Err(PluginError::Unsupported(format!(
                "plugin `{}`: wasm components; build a core module (wasm32-unknown-unknown)",
                spec.id
            ))),
            (PluginKind::Wasm, other) => wrong(format!("kind = \"wasm\" but the file is {other}")),
        }
    }

    fn build(&self, spec: &PluginSpec, target: Target, crate_dir: PathBuf, label: String) -> Result<Artifact> {
        self.builder.build(&BuildRequest {
            id: spec.id.clone(),
            crate_dir,
            target,
            features: spec.features.clone(),
            no_default_features: spec.no_default_features,
            source_label: label,
            in_process: spec.kind == PluginKind::Native && spec.effective_isolation() == Isolation::InProcess,
            crate_type: None,
        })
    }

    fn instantiate(&self, spec: &PluginSpec, artifact: &Artifact) -> Result<Box<dyn Plugin>> {
        let config = spec.config_json()?;
        let sink = tracing_sink(&spec.id);
        let plugin: Box<dyn Plugin> = match (spec.kind, spec.effective_isolation()) {
            (PluginKind::Native, Isolation::InProcess) => {
                // Cache entries are immutable and named by their key: map them in
                // place. A user's file may be rebuilt while loaded, so it is mapped
                // through a copy named by its hash.
                let shadow = self.cache().shadow_dir();
                let placement = match artifact.key {
                    Some(_) => Placement::InPlace { copies: Some(&shadow) },
                    None => Placement::Copy { dir: &shadow, tag: &artifact.sha256 },
                };
                Box::new(NativePlugin::load(&spec.id, &artifact.path, placement, &config, sink)?)
            }
            (PluginKind::Native, Isolation::Process) => {
                let mut iso = self.opts.isolation.clone();
                if let Some(t) = spec.call_timeout() {
                    iso.call_timeout = t;
                }
                let mut sandbox = spec.sandbox.clone();
                sandbox.resolve(&self.opts.base_dir);
                Box::new(IsolatedPlugin::spawn(&spec.id, &artifact.path, &config, iso, sandbox, sink)?)
            }
            (PluginKind::Wasm, _) => {
                let engine = self.engine(spec.engine.as_deref())?;
                let wasm = std::fs::read(&artifact.path).map_err(io_err(&artifact.path))?;
                let module = self.modules.compile(&engine, &wasm)?;
                Box::new(WasmPlugin::with_module(&spec.id, engine.name(), module, spec.limits.clone(), &config, sink)?)
            }
        };
        if let Some(req) = &spec.version
            && !req.matches(&plugin.info().version)
        {
            return Err(PluginError::VersionMismatch {
                id: spec.id.clone(),
                required: req.to_string(),
                found: plugin.info().version.to_string(),
            });
        }
        Ok(plugin)
    }

    /// Build if needed and load `spec`. Fails if the id is already loaded.
    pub fn load(&mut self, spec: PluginSpec) -> Result<&PluginInfo> {
        if self.plugins.contains_key(&spec.id) {
            return Err(PluginError::AlreadyLoaded(spec.id));
        }
        let _span = tracing::info_span!("load", plugin = %spec.id, kind = ?spec.kind).entered();
        let started = Instant::now();
        let res = self.resolve(&spec).and_then(|(a, e)| self.instantiate(&spec, &a).map(|p| (a, e, p)));
        match res {
            Ok((artifact, entry, plugin)) => {
                self.pin(entry);
                tracing::info!(
                    backend = %plugin.info().backend, cached = artifact.cached,
                    elapsed_ms = started.elapsed().as_millis() as u64, "plugin ready"
                );
                let id = spec.id.clone();
                self.plugins.insert(id.clone(), Loaded { spec, artifact, plugin });
                Ok(self.plugins[&id].plugin.info())
            }
            Err(e) => {
                tracing::error!(error = %e, "plugin failed to load");
                Err(e)
            }
        }
    }

    /// Load every enabled plugin; one failure never stops the others.
    pub fn load_all(&mut self, specs: impl IntoIterator<Item = PluginSpec>) -> Vec<(String, Result<()>)> {
        specs
            .into_iter()
            .filter(|s| s.enabled)
            .map(|s| {
                let id = s.id.clone();
                let r = self.load(s).map(|_| ());
                (id, r)
            })
            .collect()
    }

    /// Unload a plugin (instance destroyed, library closed / process stopped)
    /// and report whether its code actually left the host.
    pub fn unload(&mut self, id: &str) -> Result<Release> {
        let loaded = self.plugins.remove(id).ok_or_else(|| PluginError::NotFound(id.to_owned()))?;
        Ok(self.retire(id, loaded))
    }

    fn retire(&mut self, id: &str, loaded: Loaded) -> Release {
        let release = loaded.plugin.close();
        if !release.is_released() {
            self.leaked += 1;
            tracing::warn!(
                plugin = id,
                leaked_images = self.leaked,
                "library still mapped after unload; its code stays in this process until exit \
                 (isolation = \"process\" guarantees unloading)"
            );
        }
        release
    }

    /// Native libraries that stayed mapped after being unloaded or reloaded.
    pub fn leaked_images(&self) -> usize {
        self.leaked
    }

    /// Rebuild if sources changed and swap in a fresh instance. The old
    /// instance keeps serving, and keeps its lockfile pin, if the new one
    /// fails to build or load.
    pub fn reload(&mut self, id: &str) -> Result<&PluginInfo> {
        self.refresh(id, true)?;
        Ok(self.plugins[id].plugin.info())
    }

    /// `reload`; unless `force`, an artifact with the same bytes as the
    /// loaded one swaps nothing. Returns whether the instance was swapped.
    fn refresh(&mut self, id: &str, force: bool) -> Result<bool> {
        let spec = self.plugins.get(id).ok_or_else(|| PluginError::NotFound(id.to_owned()))?.spec.clone();
        let _span = tracing::info_span!("reload", plugin = %id).entered();
        let (artifact, entry) = self.resolve(&spec)?;
        if !force && artifact.sha256 == self.plugins[id].artifact.sha256 {
            tracing::debug!("artifact unchanged; nothing to reload");
            return Ok(false);
        }
        let plugin = self.instantiate(&spec, &artifact)?;
        self.pin(entry);
        if let Some(old) = self.plugins.insert(id.to_owned(), Loaded { spec, artifact, plugin }) {
            self.retire(id, old);
        }
        tracing::info!("plugin reloaded");
        Ok(true)
    }

    /// Watch the sources and artifacts of the plugins loaded now; plugins
    /// loaded later are not watched. Nothing is watched unless this is called.
    pub fn watch(&self, opts: &WatchOptions) -> Result<Watcher> {
        let mut ignore = vec![self.opts.cache_dir.clone()];
        ignore.extend(self.opts.lock_path.clone());
        let mut watches = Vec::new();
        for (id, l) in &self.plugins {
            let resolved = match &l.spec.source {
                Source::Artifact(p) | Source::Path(p) => self.resolve_path(p),
                Source::Git(_) => PathBuf::new(),
            };
            match Watch::for_source(id, &l.spec.source, &resolved, &ignore)? {
                Some(w) => watches.push(w),
                None => tracing::debug!(plugin = %id, "git source: nothing local to watch"),
            }
        }
        Watcher::new(watches, opts)
    }

    /// Reload the plugins `watcher` saw change, without waiting for more.
    /// A plugin whose rebuilt artifact has the same bytes is left alone, so a
    /// save that changes nothing does not reload. Returns every plugin that
    /// was swapped or failed; a failed one keeps its old instance.
    pub fn reload_changed(&mut self, watcher: &mut Watcher) -> Vec<(String, Result<()>)> {
        let mut out = Vec::new();
        for id in watcher.take() {
            if !self.plugins.contains_key(&id) {
                continue;
            }
            match self.refresh(&id, false) {
                Ok(false) => {}
                Ok(true) => out.push((id, Ok(()))),
                Err(e) => {
                    tracing::warn!(plugin = %id, error = %e, "reload after a change failed; the old instance keeps serving");
                    out.push((id, Err(e)));
                }
            }
        }
        out
    }

    /// Bind plugin `id` for typed calls through an `#[interface]` client:
    /// `GreeterClient::new(manager.bind("hello")?).greet("world")`.
    pub fn bind(&mut self, id: &str) -> Result<Bound<'_>> {
        if !self.plugins.contains_key(id) {
            return Err(PluginError::NotFound(id.to_owned()));
        }
        Ok(Bound { manager: self, id: id.to_owned() })
    }

    /// Methods exported by plugin `id`, as reported by `plugwright.describe`.
    pub fn describe(&mut self, id: &str) -> Result<Vec<MethodInfo>> {
        crate::client::describe(&mut self.bind(id)?)
    }

    /// Call `method` on plugin `id`; every call is logged with its outcome.
    pub fn call(&mut self, id: &str, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        let loaded = self.plugins.get_mut(id).ok_or_else(|| PluginError::NotFound(id.to_owned()))?;
        let started = Instant::now();
        let res = loaded.plugin.call(method, input);
        let elapsed_us = started.elapsed().as_micros() as u64;
        match &res {
            Ok(out) => tracing::info!(
                plugin = id,
                method,
                bytes_in = input.len(),
                bytes_out = out.len(),
                elapsed_us,
                "call ok"
            ),
            Err(e) => {
                tracing::warn!(plugin = id, method, bytes_in = input.len(), elapsed_us, error = %e, "call failed")
            }
        }
        res
    }

    pub fn call_json<I: Serialize + ?Sized, O: DeserializeOwned>(
        &mut self,
        id: &str,
        method: &str,
        input: &I,
    ) -> Result<O> {
        let out = self.call(id, method, &serde_json::to_vec(input)?)?;
        Ok(serde_json::from_slice(&out)?)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut (dyn Plugin + 'static)> {
        self.plugins.get_mut(id).map(|l| l.plugin.as_mut())
    }

    /// `(id, info, artifact)` for every loaded plugin.
    pub fn list(&self) -> impl Iterator<Item = (&str, &PluginInfo, &Artifact)> {
        self.plugins.iter().map(|(id, l)| (id.as_str(), l.plugin.info(), &l.artifact))
    }

    pub fn lockfile(&self) -> &Lockfile {
        &self.lock
    }

    fn save_lock(&self) {
        if let Some(p) = &self.opts.lock_path
            && let Err(e) = self.lock.save(p)
        {
            tracing::warn!(error = %e, "cannot write lockfile");
        }
    }

    /// Drop cached artifacts not referenced by the lockfile, compiled Wasm of
    /// modules it does not reference, and copies of libraries that nothing has
    /// mapped any more.
    pub fn gc_cache(&self) -> Result<usize> {
        let keep: HashSet<String> = self.lock.plugins.iter().filter_map(|e| e.cache_key.clone()).collect();
        let copies = crate::native::prune_copies(&self.cache().shadow_dir());
        let modules: HashSet<String> = self.lock.plugins.iter().map(|e| e.sha256.clone()).collect();
        let compiled = self.modules.prune(&modules);
        tracing::debug!(copies, compiled, "unused library copies and compiled wasm removed");
        self.cache().gc(&keep)
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        // Deterministic order: unload in reverse id order.
        while let Some((id, loaded)) = self.plugins.pop_last() {
            let release = loaded.plugin.close();
            tracing::debug!(plugin = %id, %release, "dropped by manager");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheEntry;
    use crate::wasm::{WasmError, WasmModule};

    const CORE_WASM: &[u8] = b"\0asm\x01\0\0\0";
    const COMPONENT: &[u8] = b"\0asm\x0d\0\x01\0";
    const ARCHIVE: &[u8] = b"!<arch>\n";
    /// sha256 of `b"just text"`.
    const TEXT_SHA: &str = "7f5dde76c8464594732baeae1e0171af7955005e488790245ed52949a2c7841c";

    fn manager(dir: &Path) -> PluginManager {
        PluginManager::new(ManagerOptions::new(dir)).unwrap()
    }

    fn file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn artifact(id: &str, kind: PluginKind, path: impl Into<PathBuf>) -> PluginSpec {
        let mut s = PluginSpec::new(id, kind, Source::Artifact(path.into()));
        if kind == PluginKind::Native {
            // Never spawn a runner from a unit test.
            s.isolation = Some(Isolation::InProcess);
        }
        s
    }

    fn not_a_plugin(r: Result<&PluginInfo>) -> String {
        match r {
            Err(PluginError::NotAPlugin { reason, .. }) => reason,
            Err(e) => panic!("expected NotAPlugin, got {e:?}"),
            Ok(i) => panic!("expected NotAPlugin, loaded {}", i.name),
        }
    }

    /// Refuses every module, naming itself, so tests can see which engine ran.
    struct RefusingEngine(&'static str);

    impl WasmEngine for RefusingEngine {
        fn name(&self) -> &str {
            "refusing"
        }
        fn compile(&self, _: &[u8]) -> std::result::Result<Arc<dyn WasmModule>, WasmError> {
            Err(WasmError::Compile(self.0.into()))
        }
    }

    #[test]
    fn new_creates_the_cache_and_an_in_memory_lock() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager(dir.path());
        assert_eq!(m.cache().root(), dir.path().join(".plugwright"));
        for sub in ["artifacts", "target", "git", "loaded", "meta"] {
            assert!(dir.path().join(".plugwright").join(sub).is_dir(), "{sub}");
        }
        assert_eq!(m.lockfile(), &Lockfile { version: Lockfile::VERSION, plugins: vec![] });
        assert_eq!(m.list().count(), 0);
        assert_eq!(m.leaked_images(), 0);
        let engines: Vec<_> = m.engines().collect();
        assert_eq!(engines, builtin_engines().iter().map(|e| e.name().to_owned()).collect::<Vec<_>>());
    }

    #[test]
    fn new_reads_the_lockfile_and_rejects_a_corrupt_one() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("plugwright.lock");
        let mut o = ManagerOptions::new(dir.path());
        o.lock_path = Some(lock.clone());
        assert!(PluginManager::new(o.clone()).unwrap().lockfile().plugins.is_empty(), "a missing lock is empty");
        assert!(!lock.exists(), "creating a manager does not write the lock");

        let entry =
            LockEntry { id: "a".into(), source: "s".into(), git_sha: None, cache_key: None, sha256: "h".into() };
        Lockfile { version: Lockfile::VERSION, plugins: vec![entry.clone()] }.save(&lock).unwrap();
        assert_eq!(PluginManager::new(o.clone()).unwrap().lockfile().get("a"), Some(&entry));

        std::fs::write(&lock, "version = [").unwrap();
        assert!(matches!(PluginManager::new(o), Err(PluginError::Manifest(_))));
    }

    #[test]
    fn options_from_manifest_resolve_against_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugwright.toml");
        std::fs::write(
            &path,
            "[settings]\ncache_dir = \"c\"\nlocked = true\noffline = true\nrelease = false\n\
             allow_local_git = true\nrunner = \"bin/runner\"\nmax_restarts = 9\n\
             [settings.build]\nthreads = 2",
        )
        .unwrap();
        let (mgr, m) = PluginManager::from_manifest(&path).unwrap();
        let o = ManagerOptions::from_manifest(&m);
        assert_eq!(o.cache_dir, dir.path().join("c"));
        assert_eq!(mgr.cache().root(), dir.path().join("c"));
        assert_eq!(o.base_dir, dir.path());
        assert!(o.build.locked && o.build.offline && !o.build.release && o.allow_local_git);
        assert_eq!(o.build.accel.threads, Some(2));
        assert_eq!(o.isolation.max_restarts, 9);
        assert!(matches!(&o.isolation.runner, RunnerCommand::Path(p) if *p == dir.path().join("bin/runner")));
        assert_eq!(o.lock_path, Some(dir.path().join("plugwright.lock")));
    }

    #[test]
    fn operations_on_unknown_ids_are_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manager(dir.path());
        let nf = |r: Result<()>| matches!(r, Err(PluginError::NotFound(id)) if id == "ghost");
        assert!(nf(m.bind("ghost").map(|_| ())));
        assert!(nf(m.describe("ghost").map(|_| ())));
        assert!(nf(m.call("ghost", "m", b"").map(|_| ())));
        assert!(nf(m.call_json::<_, serde_json::Value>("ghost", "m", &1).map(|_| ())));
        assert!(nf(m.unload("ghost").map(|_| ())));
        assert!(nf(m.reload("ghost").map(|_| ())));
        assert!(m.get_mut("ghost").is_none());
    }

    #[test]
    fn native_artifact_that_is_not_a_shared_library_is_not_a_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manager(dir.path());
        // Relative paths resolve against the base dir; the name means nothing.
        file(dir.path(), "notes.dll", b"just text");
        let reason = not_a_plugin(m.load(artifact("t", PluginKind::Native, "notes.dll")));
        assert_eq!(reason, "not a shared library");
        file(dir.path(), "module.so", CORE_WASM);
        let reason = not_a_plugin(m.load(artifact("w", PluginKind::Native, "module.so")));
        assert!(reason.contains("Wasm module") && reason.contains("kind = \"wasm\""), "{reason}");
        // The same check runs before a runner process would be spawned.
        let mut iso = artifact("p", PluginKind::Native, "notes.dll");
        iso.isolation = Some(Isolation::Process);
        assert_eq!(not_a_plugin(m.load(iso)), "not a shared library");
        assert_eq!(m.list().count(), 0, "failed loads leave nothing behind");
    }

    #[test]
    fn wasm_artifact_with_the_wrong_format_is_rejected_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manager(dir.path());
        let text = file(dir.path(), "plugin.wasm", b"just text");
        let reason = not_a_plugin(m.load(artifact("t", PluginKind::Wasm, &text)));
        assert_eq!(reason, "kind = \"wasm\" but the file is unknown format");
        let archive = file(dir.path(), "lib.wasm", ARCHIVE);
        let reason = not_a_plugin(m.load(artifact("a", PluginKind::Wasm, &archive)));
        assert_eq!(reason, "kind = \"wasm\" but the file is static library (ar archive)");
        let component = file(dir.path(), "c.wasm", COMPONENT);
        let err = m.load(artifact("c", PluginKind::Wasm, &component)).unwrap_err();
        assert!(matches!(&err, PluginError::Unsupported(m) if m.contains("wasm components")), "{err}");
    }

    #[test]
    fn sha256_pin_mismatch_is_an_integrity_error() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("plugwright.lock");
        let mut o = ManagerOptions::new(dir.path());
        o.lock_path = Some(lock.clone());
        let mut m = PluginManager::new(o).unwrap();
        let path = file(dir.path(), "notes.dll", b"just text");
        assert_eq!(sha256_file(&path).unwrap(), TEXT_SHA);

        let mut s = artifact("p", PluginKind::Native, &path);
        s.sha256 = Some("0".repeat(64));
        match m.load(s.clone()) {
            Err(PluginError::Integrity { path: p, expected, actual }) => {
                assert_eq!((p, expected, actual), (path.clone(), "0".repeat(64), TEXT_SHA.to_owned()));
            }
            other => panic!("expected Integrity, got {:?}", other.map(|i| i.name.clone())),
        }
        assert!(m.lockfile().get("p").is_none() && !lock.exists(), "a rejected artifact is not pinned");

        // The pin is case-insensitive; once it matches, the content check runs.
        s.sha256 = Some(TEXT_SHA.to_uppercase());
        not_a_plugin(m.load(s));
    }

    #[test]
    fn missing_artifact_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manager(dir.path());
        let err = m.load(artifact("x", PluginKind::Wasm, "nope.wasm")).unwrap_err();
        assert!(matches!(&err, PluginError::Io { path, .. } if *path == dir.path().join("nope.wasm")), "{err}");
    }

    #[test]
    fn prepare_pins_a_prebuilt_artifact_in_the_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("plugwright.lock");
        let mut o = ManagerOptions::new(dir.path());
        o.lock_path = Some(lock.clone());
        let mut m = PluginManager::new(o).unwrap();
        file(dir.path(), "m.wasm", CORE_WASM);
        let a = m.prepare(&artifact("w", PluginKind::Wasm, "m.wasm")).unwrap();
        assert_eq!((a.path, a.key, a.cached), (dir.path().join("m.wasm"), None, true));
        let e = Lockfile::load(&lock).unwrap().get("w").cloned().unwrap();
        assert_eq!((e.source.as_str(), e.cache_key, e.git_sha), ("artifact+m.wasm", None, None));
        assert_eq!(e.sha256, a.sha256);

        let mut bad = artifact("bad id", PluginKind::Wasm, "m.wasm");
        assert!(matches!(m.prepare(&bad), Err(PluginError::Manifest(_))), "specs are validated");
        bad.id = "w2".into();
        bad.link_args = vec!["-lz".into()];
        assert!(matches!(m.prepare(&bad), Err(PluginError::Manifest(_))));
    }

    #[test]
    fn only_artifacts_that_load_are_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("plugwright.lock");
        let mut o = ManagerOptions::new(dir.path());
        o.lock_path = Some(lock.clone());
        let mut m = PluginManager::new(o).unwrap();
        m.register_engine(Arc::new(RefusingEngine("no")));
        file(dir.path(), "m.wasm", CORE_WASM);
        let mut s = artifact("w", PluginKind::Wasm, "m.wasm");
        s.engine = Some("refusing".into());
        assert!(m.load(s.clone()).is_err());
        assert!(m.lockfile().get("w").is_none() && !lock.exists(), "a plugin that failed to load is not pinned");
        // An explicit build pins without loading.
        m.prepare(&s).unwrap();
        assert!(Lockfile::load(&lock).unwrap().get("w").is_some());
    }

    #[test]
    fn wasm_engines_are_chosen_by_name_and_replaced_on_register() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manager(dir.path());
        file(dir.path(), "m.wasm", CORE_WASM);
        let mut s = artifact("w", PluginKind::Wasm, "m.wasm");
        s.engine = Some("refusing".into());
        let err = m.load(s.clone()).unwrap_err();
        assert!(matches!(&err, PluginError::Unsupported(msg) if msg.contains("`refusing` is not registered")), "{err}");

        let before = m.engines().count();
        m.register_engine(Arc::new(RefusingEngine("first")));
        m.register_engine(Arc::new(RefusingEngine("second")));
        assert_eq!(m.engines().count(), before + 1, "same name replaces");
        let err = m.load(s).unwrap_err();
        assert!(matches!(&err, PluginError::Wasm(WasmError::Compile(msg)) if msg == "second"), "{err}");
    }

    #[test]
    fn load_all_skips_disabled_specs_and_survives_failures() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manager(dir.path());
        file(dir.path(), "t.wasm", b"text");
        let mut off = artifact("off", PluginKind::Wasm, "t.wasm");
        off.enabled = false;
        let res = m.load_all([
            artifact("a", PluginKind::Wasm, "t.wasm"),
            off,
            artifact("b", PluginKind::Wasm, "missing.wasm"),
            artifact("c", PluginKind::Wasm, "t.wasm"),
        ]);
        let ids: Vec<_> = res.iter().map(|(id, r)| (id.as_str(), r.is_err())).collect();
        assert_eq!(ids, [("a", true), ("b", true), ("c", true)]);
    }

    #[test]
    fn gc_cache_drops_entries_the_lockfile_does_not_reference() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager(dir.path());
        let src = file(dir.path(), "lib.so", b"bytes");
        for k in ["aa11", "bb22"] {
            let entry = CacheEntry {
                key: k.into(),
                id: "x".into(),
                file: String::new(),
                sha256: String::new(),
                size: 0,
                source: "test".into(),
                target: "host".into(),
                built_at_unix: 0,
            };
            m.cache().store(k, &src, entry).unwrap();
        }
        assert_eq!(m.gc_cache().unwrap(), 2);
        assert!(m.cache().lookup("aa11").unwrap().is_none());
    }
}
