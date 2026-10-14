//! Builds plugin crates with cargo and serves results from the [`Cache`].
//!
//! Cache layers: the plugwright key (L1) lets a hit skip cargo entirely; on a
//! miss cargo builds into the shared `cache/target` dir (L2, cargo's own
//! incremental fingerprints).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::accel::{Accel, Plan, PlanInput, Wrapper};
use crate::cache::{Artifact, Cache, CacheEntry, hex, sha256_hex};
use plugwright_core::error::{PluginError, Result, io_err};

/// Bump when the key derivation or the store layout changes.
const CACHE_SCHEMA: &str = "plugwright-cache-v1";
/// Refuse to fingerprint absurdly large trees.
const MAX_FINGERPRINT_FILES: usize = 50_000;
const STDERR_TAIL: usize = 40;

/// What to compile for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    /// The host triple; produces `.so` / `.dylib` / `.dll`, or a static
    /// library (`.a` / `.lib`) for `crate-type = ["staticlib"]`.
    Native,
    /// `wasm32-unknown-unknown`; produces `.wasm`.
    Wasm,
}

impl Target {
    pub fn triple(self) -> Option<&'static str> {
        match self {
            Target::Native => None,
            Target::Wasm => Some("wasm32-unknown-unknown"),
        }
    }

    pub fn artifact_ext(self) -> &'static str {
        match self {
            Target::Native => std::env::consts::DLL_EXTENSION,
            Target::Wasm => "wasm",
        }
    }
}

/// Global build knobs.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// cargo executable (defaults to `$CARGO` or `cargo`).
    pub cargo: PathBuf,
    /// Pass `--locked`.
    pub locked: bool,
    /// Pass `--offline`.
    pub offline: bool,
    /// Build with `--release`.
    pub release: bool,
    /// Toolchain, compiler cache, linker, codegen backend, profile tuning.
    pub accel: Accel,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            cargo: std::env::var_os("CARGO").map(PathBuf::from).unwrap_or_else(|| "cargo".into()),
            locked: false,
            offline: false,
            release: true,
            accel: Accel::default(),
        }
    }
}

/// One crate to build.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub id: String,
    pub crate_dir: PathBuf,
    pub target: Target,
    pub features: Vec<String>,
    pub no_default_features: bool,
    /// Human-readable source description stored in the cache entry.
    pub source_label: String,
    /// The artifact will be loaded into the host process, so plugin panics
    /// must unwind (rules out Cranelift, which aborts).
    pub in_process: bool,
}

pub struct Builder {
    cache: Cache,
    opts: BuildOptions,
}

impl Builder {
    pub fn new(cache: Cache, opts: BuildOptions) -> Self {
        Builder { cache, opts }
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    pub fn options(&self) -> &BuildOptions {
        &self.opts
    }

    /// Compute the cache key for `req` without building.
    pub fn cache_key(&self, req: &BuildRequest) -> Result<String> {
        self.key_and_plan(req)?.0.key()
    }

    /// Resolve the build accelerators for `req` (see [`Accel::plan`]).
    pub fn plan(&self, req: &BuildRequest) -> Result<Plan> {
        let rustc = self.rustc_vv(&req.crate_dir, self.opts.accel.toolchain.as_deref())?;
        self.plan_with(req, &rustc)
    }

    fn plan_with(&self, req: &BuildRequest, rustc_vv: &str) -> Result<Plan> {
        let input = PlanInput {
            rustc_vv,
            toolchain: self.opts.accel.toolchain.as_deref(),
            dir: &req.crate_dir,
            release: self.opts.release,
            target: req.target,
            needs_unwind: req.in_process,
        };
        self.opts.accel.plan(input).map_err(|message| PluginError::build(&req.id, message))
    }

    fn key_and_plan(&self, req: &BuildRequest) -> Result<(Keyed, Plan)> {
        manifest_path(req)?;
        let rustc = self.rustc_vv(&req.crate_dir, self.opts.accel.toolchain.as_deref())?;
        let plan = self.plan_with(req, &rustc)?;
        let (fp, lockfile) = self.fingerprint(req)?;
        let mut h = Sha256::new();
        let mut put = |k: &str, v: &str| {
            h.update(k.as_bytes());
            h.update(b"=");
            h.update(v.as_bytes());
            h.update(b"\n");
        };
        put("schema", CACHE_SCHEMA);
        put("plugwright", env!("CARGO_PKG_VERSION"));
        put("abi", &plugwright_abi::ABI_VERSION.to_string());
        put("rustc", &rustc);
        put("target", req.target.triple().unwrap_or("host"));
        put("profile", if self.opts.release { "release" } else { "dev" });
        let features: BTreeSet<&str> = req.features.iter().map(String::as_str).collect();
        put("features", &features.into_iter().collect::<Vec<_>>().join(","));
        put("no_default_features", &req.no_default_features.to_string());
        for (var, value) in output_env() {
            put(&var, &value);
        }
        put("RUSTC_WRAPPER", &wrapper_key(&plan.wrapper, std::env::var_os("RUSTC_WRAPPER").as_deref()));
        put("accel", &plan.key_material());
        put("source", &fp);
        Ok((Keyed { base: hex(&h.finalize()), lockfile }, plan))
    }

    /// Build `req` or return the cached artifact.
    pub fn build(&self, req: &BuildRequest) -> Result<Artifact> {
        let _span = tracing::info_span!("build", plugin = %req.id, target = ?req.target).entered();
        let started = Instant::now();
        let (keyed, plan) = self.key_and_plan(req)?;
        let key = keyed.key()?;
        if let Some(a) = self.cache.lookup(&key)? {
            tracing::info!(key = &key[..12], "cache hit");
            return Ok(a);
        }
        let _lock = self.cache.lock()?;
        // Another process may have finished the same build while we waited.
        if let Some(a) = self.cache.lookup(&key)? {
            tracing::info!(key = &key[..12], "cache hit after lock");
            return Ok(a);
        }
        tracing::info!(key = &key[..12], accel = ?plan.applied, "cache miss; running cargo");
        let built = self.run_cargo(req, &plan)?;
        // cargo creates a missing `Cargo.lock` and updates a stale one, and
        // builds with what it wrote: that is the lockfile this artifact is for.
        let key = keyed.key()?;
        let entry = CacheEntry {
            key: key.clone(),
            id: req.id.clone(),
            file: String::new(),
            sha256: String::new(),
            size: 0,
            source: req.source_label.clone(),
            target: req.target.triple().unwrap_or("host").into(),
            built_at_unix: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        };
        let a = self.cache.store(&key, &built, entry)?;
        tracing::info!(
            key = &key[..12],
            sha256 = &a.sha256[..12],
            elapsed_ms = started.elapsed().as_millis() as u64,
            "plugin built and cached"
        );
        Ok(a)
    }

    /// `cargo`, or `cargo +<toolchain>` through the rustup proxy.
    fn cargo_command(&self) -> Command {
        match &self.opts.accel.toolchain {
            Some(tc) => {
                let mut cmd = Command::new("cargo");
                cmd.arg(format!("+{tc}")).env_remove("RUSTC");
                cmd
            }
            None => Command::new(&self.opts.cargo),
        }
    }

    fn run_cargo(&self, req: &BuildRequest, plan: &Plan) -> Result<PathBuf> {
        // Absolute: cargo runs in the crate directory, so that its
        // rust-toolchain file applies (as for `rustc -vV`).
        let absolute = |p: &Path| std::path::absolute(p).map_err(io_err(p));
        let manifest = absolute(&manifest_path(req)?)?;
        let target_dir = absolute(&self.cache.target_dir())?;
        let mut cmd = self.cargo_command();
        cmd.current_dir(&req.crate_dir);
        cmd.arg("build")
            .arg("--lib")
            .arg("--message-format=json-render-diagnostics")
            .arg("--manifest-path")
            .arg(&manifest)
            .arg("--target-dir")
            .arg(&target_dir)
            // A panic escaping `extern "C"` aborts the host; force unwinding so
            // the SDK's catch_unwind works even if the plugin's profile says abort.
            .args(["--config", "profile.release.panic=\"unwind\""])
            .args(["--config", "profile.dev.panic=\"unwind\""]);
        if self.opts.release {
            cmd.arg("--release");
        }
        if let Some(t) = req.target.triple() {
            cmd.args(["--target", t]);
        }
        if self.opts.locked {
            cmd.arg("--locked");
        }
        if self.opts.offline {
            cmd.arg("--offline");
        }
        if !req.features.is_empty() {
            cmd.arg("--features").arg(req.features.join(","));
        }
        if req.no_default_features {
            cmd.arg("--no-default-features");
        }
        plan.apply(&mut cmd);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        tracing::debug!(?cmd, "spawning cargo");
        let mut child =
            cmd.spawn().map_err(|e| PluginError::build(&req.id, format!("cannot run {:?}: {e}", self.opts.cargo)))?;

        let stderr = child.stderr.take().expect("piped");
        let tail = std::thread::spawn(move || {
            let mut tail = VecDeque::with_capacity(STDERR_TAIL);
            for line in BufReader::new(stderr).lines().map_while(std::io::Result::ok) {
                tracing::trace!(target: "plugwright::cargo", "{line}");
                if tail.len() == STDERR_TAIL {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
            tail.into_iter().collect::<Vec<_>>().join("\n")
        });

        let want_manifest = std::fs::canonicalize(&manifest).unwrap_or(manifest.clone());
        let ext = req.target.artifact_ext();
        let (mut found, mut staticlib) = (None, None);
        let stdout = child.stdout.take().expect("piped");
        for line in BufReader::new(stdout).lines().map_while(std::io::Result::ok) {
            let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) else { continue };
            if msg.reason != "compiler-artifact" {
                continue;
            }
            let same = msg.manifest_path.as_deref().and_then(|p| std::fs::canonicalize(p).ok()).as_ref()
                == Some(&want_manifest);
            let has = |ty: &str| msg.target.as_ref().is_some_and(|t| t.crate_types.iter().any(|c| c == ty));
            if same && has("cdylib") {
                found = msg.filenames.iter().find(|f| f.extension().is_some_and(|e| e == ext)).cloned();
            }
            // A staticlib-only crate is relinked into a shared library later.
            if same && req.target == Target::Native && has("staticlib") {
                staticlib = msg.filenames.into_iter().find(|f| f.extension().is_some_and(|e| e == "a" || e == "lib"));
            }
        }
        let status = child.wait().map_err(|e| PluginError::build(&req.id, e.to_string()))?;
        let tail = tail.join().unwrap_or_default();
        if !status.success() {
            return Err(PluginError::build(&req.id, format!("cargo failed ({status}):\n{tail}")));
        }
        found.or(staticlib).ok_or_else(|| {
            PluginError::build(
                &req.id,
                format!(
                    "no cdylib `.{ext}` (or staticlib) artifact produced; set `crate-type = [\"cdylib\"]` in [lib]"
                ),
            )
        })
    }

    /// Hash every file of every *path* package the plugin depends on
    /// (including itself), the workspace manifests and every cargo config file
    /// that applies; see [`crate::sources`]. The lockfile is returned to be
    /// hashed on its own.
    fn fingerprint(&self, req: &BuildRequest) -> Result<(String, PathBuf)> {
        let sources = crate::sources::sources(&req.crate_dir).map_err(|e| PluginError::build(&req.id, e))?;
        let skip: Vec<PathBuf> = [self.cache.root().to_path_buf()]
            .into_iter()
            .chain(std::fs::canonicalize(self.cache.root()).ok())
            .collect();
        let mut files = BTreeSet::new();
        for root in &sources.packages {
            if root.is_dir() {
                collect_files(root, &skip, &mut files)?;
            }
        }
        if files.len() > MAX_FINGERPRINT_FILES {
            return Err(PluginError::build(
                &req.id,
                format!("source tree too large to fingerprint ({} files)", files.len()),
            ));
        }
        files.extend(sources.files);
        files.remove(&sources.lockfile);
        let mut h = Sha256::new();
        for f in &files {
            hash_input(&mut h, f)?;
        }
        Ok((hex(&h.finalize()), sources.lockfile))
    }

    /// `rustc -vV` as seen from `dir`, remembered in `meta/`. Asking costs a
    /// process (20–100 ms through a rustup or mise proxy); the answer only
    /// changes when what selects the toolchain or the compiler binary does.
    pub(crate) fn rustc_vv(&self, dir: &Path, toolchain: Option<&str>) -> Result<String> {
        let dir = std::path::absolute(dir).map_err(io_err(dir))?;
        let mut selector = format!("{}\n{toolchain:?}\n", dir.display());
        for (k, v) in std::env::vars_os() {
            let k = k.to_string_lossy();
            if TOOLCHAIN_ENV.contains(&&*k) || TOOLCHAIN_ENV_PREFIXES.iter().any(|p| k.starts_with(p)) {
                selector.push_str(&format!("{k}={}\n", v.to_string_lossy()));
            }
        }
        let memo_path =
            self.cache.root().join("meta").join(format!("rustc-{}.json", &sha256_hex(selector.as_bytes())[..32]));
        if let Ok(raw) = std::fs::read(&memo_path)
            && let Ok(memo) = serde_json::from_slice::<RustcMemo>(&raw)
            && memo.selector == selector
            && memo.is_fresh()
        {
            return Ok(memo.vv);
        }
        // Stamps first: a change while rustc runs leaves a memo that is already stale.
        let mut watched: Vec<Stamp> = toolchain_files(&dir).iter().map(|p| stamp(p)).collect();
        let sysroot = rustc_command(toolchain)
            .args(["--print", "sysroot"])
            .current_dir(&dir)
            .stdin(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
        let compiler = sysroot.map(|s| s.join("bin").join(format!("rustc{}", std::env::consts::EXE_SUFFIX)));
        let vv = rustc_version(&dir, toolchain)?;
        if let Some(compiler) = compiler.filter(|c| c.is_file()) {
            watched.push(stamp(&compiler));
            let memo = RustcMemo { selector, vv: vv.clone(), watched };
            let tmp = memo_path.with_extension(format!("{}.tmp", std::process::id()));
            if std::fs::write(&tmp, serde_json::to_vec(&memo)?).is_ok() && std::fs::rename(&tmp, &memo_path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
        Ok(vv)
    }
}

/// A cache key short of the lockfile, which the build may write.
struct Keyed {
    base: String,
    lockfile: PathBuf,
}

impl Keyed {
    fn key(&self) -> Result<String> {
        let mut h = Sha256::new();
        h.update(self.base.as_bytes());
        hash_input(&mut h, &self.lockfile)?;
        Ok(hex(&h.finalize()))
    }
}

/// Path, length and content of an input file. An absent file is an input
/// too (creating it changes the build); a length no file can have marks it.
fn hash_input(h: &mut Sha256, f: &Path) -> Result<()> {
    h.update(f.to_string_lossy().as_bytes());
    match std::fs::read(f) {
        Ok(data) => {
            h.update((data.len() as u64).to_le_bytes());
            h.update(&data);
        }
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
            h.update(u64::MAX.to_le_bytes());
        }
        Err(e) => return Err(PluginError::Io { path: f.to_owned(), source: e }),
    }
    Ok(())
}

/// Environment that picks the toolchain `rustc` resolves to.
const TOOLCHAIN_ENV: [&str; 4] = ["RUSTC", "PATH", "HOME", "CARGO_HOME"];
const TOOLCHAIN_ENV_PREFIXES: [&str; 3] = ["RUSTUP_", "MISE_", "ASDF_"];

/// Files that select a toolchain for `dir`: rustup and mise/asdf
/// configuration in it and its ancestors, and their global settings.
fn toolchain_files(dir: &Path) -> Vec<PathBuf> {
    const LOCAL: [&str; 12] = [
        "rust-toolchain",
        "rust-toolchain.toml",
        "mise.toml",
        ".mise.toml",
        "mise.local.toml",
        ".mise.local.toml",
        ".mise/config.toml",
        "mise/config.toml",
        ".config/mise.toml",
        ".config/mise/config.toml",
        ".rtx.toml",
        ".tool-versions",
    ];
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let home = std::env::home_dir();
    let mut files: Vec<PathBuf> = dir.ancestors().flat_map(|d| LOCAL.map(|f| d.join(f))).collect();
    files.extend(
        env("RUSTUP_HOME").or_else(|| home.as_ref().map(|h| h.join(".rustup"))).map(|r| r.join("settings.toml")),
    );
    let config = env("XDG_CONFIG_HOME").or_else(|| home.as_ref().map(|h| h.join(".config")));
    files.extend(env("MISE_CONFIG_DIR").or_else(|| config.map(|c| c.join("mise"))).map(|m| m.join("config.toml")));
    files.extend(home.map(|h| h.join(".tool-versions")));
    files
}

/// Environment cargo reads that can change what it builds, as key material:
/// flags, profile and build settings, per-target linkers and flags.
fn output_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.to_string_lossy().into_owned())))
        .filter(|(k, _)| {
            matches!(k.as_str(), "RUSTFLAGS" | "CARGO_ENCODED_RUSTFLAGS")
                || ["CARGO_PROFILE_", "CARGO_BUILD_", "CARGO_UNSTABLE_"].iter().any(|p| k.starts_with(p))
                || (k.starts_with("CARGO_TARGET_") && !matches!(k.as_str(), "CARGO_TARGET_DIR" | "CARGO_TARGET_TMPDIR"))
        })
        .collect();
    // Unset and empty are one value for the two flag variables, as before.
    for k in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
        if !env.iter().any(|(e, _)| e == k) {
            env.push((k.to_owned(), String::new()));
        }
    }
    env.sort();
    env
}

/// The inherited `RUSTC_WRAPPER` as cache key material. sccache only replays
/// what rustc would produce, and a wrapper the plan overrides never runs, so
/// neither is keyed; any other wrapper may change the output and is.
fn wrapper_key(plan: &Wrapper, env: Option<&OsStr>) -> String {
    match (plan, env) {
        (Wrapper::Keep, Some(w)) if Path::new(w).file_stem().is_none_or(|s| s != "sccache") => {
            w.to_string_lossy().into_owned()
        }
        _ => String::new(),
    }
}

fn manifest_path(req: &BuildRequest) -> Result<PathBuf> {
    let m = req.crate_dir.join("Cargo.toml");
    if m.is_file() { Ok(m) } else { Err(PluginError::build(&req.id, format!("{} not found", m.display()))) }
}

fn collect_files(dir: &Path, skip: &[PathBuf], out: &mut BTreeSet<PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(plugwright_core::error::io_err(dir))?;
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            let n = name.to_string_lossy();
            // `.cargo/config.toml` changes the build, other dot-dirs (.git, …) do not.
            if n == "target" || (n.starts_with('.') && n != ".cargo") || skip.iter().any(|s| s == &path) {
                continue;
            }
            collect_files(&path, skip, out)?;
        } else if ft.is_file() {
            out.insert(path);
        }
        if out.len() > MAX_FINGERPRINT_FILES {
            return Ok(());
        }
    }
    Ok(())
}

/// `rustc -vV` as seen from `dir` (rustup honours `rust-toolchain.toml` there).
/// `rustc`, or `rustc +<toolchain>` through the rustup proxy.
pub(crate) fn rustc_command(toolchain: Option<&str>) -> Command {
    match toolchain {
        Some(tc) => {
            let mut cmd = Command::new("rustc");
            cmd.arg(format!("+{tc}"));
            cmd
        }
        None => Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into())),
    }
}

pub(crate) fn rustc_version(dir: &Path, toolchain: Option<&str>) -> Result<String> {
    type Memo = HashMap<(PathBuf, Option<String>), String>;
    static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);
    let k = (dir.to_owned(), toolchain.map(str::to_owned));
    if let Some(v) = memo.lock().unwrap_or_else(|p| p.into_inner()).get(&k) {
        return Ok(v.clone());
    }
    let mut cmd = rustc_command(toolchain);
    let rustc = cmd.get_program().to_owned();
    let out = cmd
        .arg("-vV")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| PluginError::build("", format!("cannot run {rustc:?} -vV: {e}")))?;
    let mut v = String::new();
    out.stdout.as_slice().read_to_string(&mut v).ok();
    memo.lock().unwrap_or_else(|p| p.into_inner()).insert(k, v.clone());
    Ok(v)
}

/// `(path, size, mtime in ns)`; size `u64::MAX` marks a missing file.
type Stamp = (PathBuf, u64, u128);

fn stamp(p: &Path) -> Stamp {
    match std::fs::metadata(p) {
        Ok(m) => {
            let mtime = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            (p.to_owned(), m.len(), mtime)
        }
        Err(_) => (p.to_owned(), u64::MAX, 0),
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RustcMemo {
    /// Guards against a hash collision in the file name.
    selector: String,
    vv: String,
    watched: Vec<Stamp>,
}

impl RustcMemo {
    fn is_fresh(&self) -> bool {
        self.watched.iter().all(|w| stamp(&w.0) == *w)
    }
}

#[derive(serde::Deserialize)]
struct CargoMessage {
    reason: String,
    manifest_path: Option<PathBuf>,
    target: Option<CargoTarget>,
    #[serde(default)]
    filenames: Vec<PathBuf>,
}

#[derive(serde::Deserialize)]
struct CargoTarget {
    #[serde(default)]
    crate_types: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set in the child process of `sccache_in_rustc_wrapper_does_not_change_the_cache_key`.
    const KEY_CHILD_ENV: &str = "PLUGWRIGHT_TEST_KEY_CRATE";

    fn scratch_crate(dir: &Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"scratch\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\ncrate-type = [\"cdylib\"]\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    }

    fn req(dir: &Path) -> BuildRequest {
        BuildRequest {
            id: "k".into(),
            crate_dir: dir.to_owned(),
            target: Target::Native,
            features: vec![],
            no_default_features: false,
            source_label: "test".into(),
            in_process: false,
        }
    }

    fn builder(cache: &Path, opts: BuildOptions) -> Builder {
        Builder::new(Cache::new(cache).unwrap(), opts)
    }

    fn key(b: &Builder, req: &BuildRequest) -> String {
        b.cache_key(req).unwrap()
    }

    #[test]
    fn targets_name_their_triple_and_extension() {
        assert_eq!(Target::Native.triple(), None);
        assert_eq!(Target::Wasm.triple(), Some("wasm32-unknown-unknown"));
        assert_eq!(Target::Native.artifact_ext(), std::env::consts::DLL_EXTENSION);
        assert_eq!(Target::Wasm.artifact_ext(), "wasm");
        let o = BuildOptions::default();
        assert!(o.release && !o.locked && !o.offline);
    }

    #[test]
    fn missing_cargo_toml_is_a_build_error() {
        let dir = tempfile::tempdir().unwrap();
        let b = builder(&dir.path().join("cache"), BuildOptions::default());
        for err in [b.cache_key(&req(dir.path())).unwrap_err(), b.build(&req(dir.path())).unwrap_err()] {
            assert!(
                matches!(&err, PluginError::Build { id, message } if id == "k" && message.contains("Cargo.toml not found")),
                "{err}"
            );
        }
    }

    #[test]
    fn cache_key_is_stable_and_independent_of_the_cache_location() {
        let dir = tempfile::tempdir().unwrap();
        let krate = dir.path().join("crate");
        scratch_crate(&krate);
        let r = req(&krate);
        let a = builder(&dir.path().join("c1"), BuildOptions::default());
        let k = key(&a, &r);
        assert_eq!(k.len(), 64);
        assert_eq!(key(&a, &r), k);
        assert_eq!(key(&builder(&dir.path().join("c2"), BuildOptions::default()), &r), k);
    }

    #[test]
    fn cache_key_changes_with_target_profile_features_and_accel() {
        let dir = tempfile::tempdir().unwrap();
        let krate = dir.path().join("crate");
        scratch_crate(&krate);
        let cache = dir.path().join("cache");
        let with = |f: &dyn Fn(&mut BuildOptions, &mut BuildRequest)| {
            let (mut o, mut r) = (BuildOptions::default(), req(&krate));
            f(&mut o, &mut r);
            key(&builder(&cache, o), &r)
        };
        let base = with(&|_, _| {});
        let changed = [
            ("wasm target", with(&|_, r| r.target = Target::Wasm)),
            ("dev profile", with(&|o, _| o.release = false)),
            ("a feature", with(&|_, r| r.features = vec!["a".into()])),
            ("no default features", with(&|_, r| r.no_default_features = true)),
            ("lto", with(&|o, _| o.accel.profile.lto = Some("fat".into()))),
            ("opt-level", with(&|o, _| o.accel.profile.opt_level = Some("1".into()))),
        ];
        for (what, k) in &changed {
            assert_ne!(k, &base, "{what} must change the key");
        }
        let distinct: BTreeSet<_> = changed.iter().map(|(_, k)| k).collect();
        assert_eq!(distinct.len(), changed.len());

        let features = |f: &[&str]| with(&|_, r| r.features = f.iter().map(|s| (*s).to_owned()).collect());
        assert_eq!(features(&["a", "b"]), features(&["b", "a", "a"]), "feature order and duplicates do not matter");
        // Neither the compiler cache nor the isolation mode (with LLVM) changes the bytes.
        assert_eq!(with(&|o, _| o.accel.sccache = crate::accel::Toggle::Off), base);
        assert_eq!(with(&|_, r| r.in_process = true), base);
    }

    #[test]
    fn cache_key_changes_with_the_toolchain() {
        let dir = tempfile::tempdir().unwrap();
        scratch_crate(dir.path());
        let default = rustc_version(dir.path(), None).unwrap();
        // Only toolchains that are installed: naming a missing one may make rustup download it.
        let Ok(list) = Command::new("rustup").args(["toolchain", "list"]).stdin(Stdio::null()).output() else {
            eprintln!("skipping: no rustup");
            return;
        };
        let other = String::from_utf8_lossy(&list.stdout)
            .lines()
            .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
            .find(|tc| rustc_version(dir.path(), Some(tc)).is_ok_and(|v| v.contains("host: ") && v != default));
        let Some(tc) = other else {
            eprintln!("skipping: no second toolchain with a different rustc");
            return;
        };
        let cache = dir.path().join(".cache");
        let mut o = BuildOptions::default();
        let base = key(&builder(&cache, o.clone()), &req(dir.path()));
        o.accel.toolchain = Some(tc.clone());
        assert_ne!(key(&builder(&cache, o), &req(dir.path())), base, "toolchain {tc}");
    }

    #[test]
    fn cache_key_follows_sources_but_not_build_output_vcs_or_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        scratch_crate(dir.path());
        // The cache inside the crate must not fingerprint itself (the memo lives there).
        let b = builder(&dir.path().join("pwcache"), BuildOptions::default());
        let r = req(dir.path());
        let base = key(&b, &r);

        for (path, data) in [("target/debug/out.so", "x"), (".git/HEAD", "ref"), ("pwcache/artifacts/junk", "j")] {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
            assert_eq!(key(&b, &r), base, "{path} is not a source");
        }

        std::fs::create_dir_all(dir.path().join(".cargo")).unwrap();
        std::fs::write(dir.path().join(".cargo/config.toml"), "[build]\n").unwrap();
        let with_config = key(&b, &r);
        assert_ne!(with_config, base, ".cargo/config.toml changes the build");

        std::fs::write(dir.path().join("src/lib.rs"), "pub fn g() {}\n").unwrap();
        let edited = key(&b, &r);
        assert_ne!(edited, with_config, "a source edit changes the key");
        std::fs::write(dir.path().join("src/new.rs"), "").unwrap();
        assert_ne!(key(&b, &r), edited, "a new file changes the key");
    }

    #[test]
    fn rustc_answer_is_remembered_until_a_watched_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let b = builder(&dir.path().join("cache"), BuildOptions::default());
        let real = b.rustc_vv(dir.path(), None).unwrap();
        assert!(real.contains("host: "), "{real}");
        let memos: Vec<PathBuf> = std::fs::read_dir(b.cache.root().join("meta"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("rustc-")))
            .collect();
        let [memo_path] = memos.as_slice() else { panic!("one memo expected: {memos:?}") };
        let mut memo: RustcMemo = serde_json::from_slice(&std::fs::read(memo_path).unwrap()).unwrap();
        assert!(memo.is_fresh());
        assert!(memo.watched.iter().any(|w| w.0.ends_with("rust-toolchain.toml") && w.1 == u64::MAX));
        let compiler = memo.watched.last().unwrap().clone();
        assert!(compiler.0.is_file() && compiler.1 != u64::MAX, "the compiler binary is watched: {compiler:?}");

        // A fresh memo answers without running rustc ...
        memo.vv = "remembered".into();
        std::fs::write(memo_path, serde_json::to_vec(&memo).unwrap()).unwrap();
        assert_eq!(b.rustc_vv(dir.path(), None).unwrap(), "remembered");
        // ... a stale one (here: the compiler binary changed) runs it again.
        memo.watched.last_mut().unwrap().2 ^= 1;
        std::fs::write(memo_path, serde_json::to_vec(&memo).unwrap()).unwrap();
        assert_eq!(b.rustc_vv(dir.path(), None).unwrap(), real);
        // Created toolchain files count: a missing file has its own stamp.
        let missing = dir.path().join("rust-toolchain.toml");
        assert_eq!(stamp(&missing).1, u64::MAX);
    }

    #[test]
    fn only_output_affecting_cargo_env_is_key_material() {
        let env = output_env();
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"RUSTFLAGS") && names.contains(&"CARGO_ENCODED_RUSTFLAGS"), "always present");
        assert!(names.windows(2).all(|w| w[0] <= w[1]), "sorted");
        // cargo sets these for test binaries; they do not change a plugin build.
        for n in names {
            assert!(!matches!(n, "CARGO_TARGET_TMPDIR" | "CARGO_MANIFEST_DIR" | "CARGO_PKG_NAME"), "{n}");
        }
    }

    #[test]
    fn only_wrappers_that_can_change_output_are_key_material() {
        fn w(p: &str) -> Option<&OsStr> {
            Some(OsStr::new(p))
        }
        assert_eq!(wrapper_key(&Wrapper::Keep, None), "");
        assert_eq!(wrapper_key(&Wrapper::Keep, w("")), "");
        assert_eq!(wrapper_key(&Wrapper::Keep, w("sccache")), "");
        assert_eq!(wrapper_key(&Wrapper::Keep, w("/opt/bin/sccache")), "");
        assert_eq!(wrapper_key(&Wrapper::Keep, w("/opt/bin/my-wrapper")), "/opt/bin/my-wrapper");
        // The plan replaces the inherited wrapper, so it never runs.
        assert_eq!(wrapper_key(&Wrapper::Disable, w("/opt/bin/my-wrapper")), "");
        assert_eq!(wrapper_key(&Wrapper::Set("/opt/bin/sccache".into()), None), "");
    }

    #[test]
    #[ignore = "child process of sccache_in_rustc_wrapper_does_not_change_the_cache_key"]
    fn print_cache_key_of_crate_from_env() {
        let Some(dir) = std::env::var_os(KEY_CHILD_ENV).map(PathBuf::from) else { return };
        let b = builder(&dir.join("pwcache"), BuildOptions::default());
        println!("cache-key={}", key(&b, &req(&dir)));
    }

    /// `RUSTC_WRAPPER` is process-global, so each key is computed in a child.
    #[test]
    fn sccache_in_rustc_wrapper_does_not_change_the_cache_key() {
        let dir = tempfile::tempdir().unwrap();
        scratch_crate(dir.path());
        let key_with = |wrapper: Option<&str>| {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args(["builder::tests::print_cache_key_of_crate_from_env", "--exact", "--ignored", "--nocapture"])
                .env(KEY_CHILD_ENV, dir.path())
                .stdin(Stdio::null());
            match wrapper {
                Some(w) => cmd.env("RUSTC_WRAPPER", w),
                None => cmd.env_remove("RUSTC_WRAPPER"),
            };
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
            stdout
                .lines()
                .find_map(|l| l.split_once("cache-key=").and_then(|(_, k)| k.get(..64)).map(str::to_owned))
                .unwrap_or_else(|| panic!("no key printed:\n{stdout}"))
        };
        let plain = key_with(None);
        assert_eq!(key_with(Some("/opt/bin/sccache")), plain);
        assert_ne!(key_with(Some("/opt/bin/my-rustc-wrapper")), plain, "other wrappers may change the output");
    }
}
