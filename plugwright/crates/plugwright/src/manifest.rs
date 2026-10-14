//! `plugwright.toml` (what to load) and `plugwright.lock` (what was resolved).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::accel::Accel;
use crate::error::{PluginError, Result, io_err};
use crate::git::{GitRef, GitSource};
use crate::sandbox::SandboxPolicy;
use crate::wasm::WasmLimits;

/// Parsed `plugwright.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default, rename = "plugin")]
    pub plugins: Vec<PluginSpec>,
    /// Directory relative paths are resolved against (the manifest's directory).
    #[serde(skip)]
    pub base_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub cache_dir: PathBuf,
    pub locked: bool,
    pub offline: bool,
    pub release: bool,
    pub allow_local_git: bool,
    /// Explicit runner binary for isolated plugins (default: re-exec the host).
    pub runner: Option<PathBuf>,
    /// Restarts allowed per minute for an isolated plugin before it is disabled.
    pub max_restarts: u32,
    /// `[settings.build]`: toolchain and build accelerators.
    pub build: Accel,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            cache_dir: ".plugwright".into(),
            locked: false,
            offline: false,
            release: true,
            allow_local_git: false,
            runner: None,
            max_restarts: 3,
            build: Accel::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    #[default]
    Native,
    Wasm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Isolation {
    /// Load into the host process: fastest, but the plugin is fully trusted —
    /// a crash kills the host and its code runs with the host's privileges.
    InProcess,
    /// Load into a sandboxed child runner process: survives segfaults and
    /// aborts. The default.
    #[default]
    Process,
}

/// Where the plugin comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A crate directory on disk.
    Path(PathBuf),
    /// A crate inside a git repository.
    Git(GitSource),
    /// A prebuilt shared library (`.so`, `.so.N`, `.dylib`, `.bundle`, `.dll`),
    /// static library (`.a`, `.lib`; relinked) or `.wasm` module. The format
    /// is detected from the content, not the name. Pin it with `sha256`.
    Artifact(PathBuf),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    path: Option<PathBuf>,
    git: Option<String>,
    rev: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    subdir: Option<PathBuf>,
    artifact: Option<PathBuf>,
}

impl TryFrom<RawSource> for Source {
    type Error = String;

    fn try_from(r: RawSource) -> std::result::Result<Self, String> {
        let git_only = r.rev.is_some() || r.branch.is_some() || r.tag.is_some() || r.subdir.is_some();
        match (r.path, r.git, r.artifact) {
            (Some(p), None, None) if !git_only => Ok(Source::Path(p)),
            (None, Some(url), None) => {
                let reference = match (r.rev, r.branch, r.tag) {
                    (Some(v), None, None) => GitRef::Rev(v),
                    (None, Some(b), None) => GitRef::Branch(b),
                    (None, None, Some(t)) => GitRef::Tag(t),
                    (None, None, None) => GitRef::DefaultBranch,
                    _ => return Err("use at most one of `rev`, `branch`, `tag`".into()),
                };
                Ok(Source::Git(GitSource { url, reference, subdir: r.subdir }))
            }
            (None, None, Some(a)) if !git_only => Ok(Source::Artifact(a)),
            (None, None, None) => Err("source needs one of `path`, `git`, `artifact`".into()),
            _ => Err("source must have exactly one of `path`, `git`, `artifact`; \
                      `rev`/`branch`/`tag`/`subdir` only apply to `git`"
                .into()),
        }
    }
}

impl<'de> Deserialize<'de> for Source {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        RawSource::deserialize(d)?.try_into().map_err(serde::de::Error::custom)
    }
}

/// One `[[plugin]]` entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSpec {
    pub id: String,
    #[serde(default)]
    pub kind: PluginKind,
    pub source: Source,
    /// Semver requirement the plugin's own version must satisfy.
    pub version: Option<semver::VersionReq>,
    /// Native plugins only; `None` means [`Isolation::default`] (process).
    #[serde(default)]
    pub isolation: Option<Isolation>,
    /// OS sandbox for `isolation = "process"`.
    #[serde(default)]
    pub sandbox: SandboxPolicy,
    /// Wasm engine name; default: the first registered engine.
    pub engine: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub no_default_features: bool,
    /// Extra linker arguments used when a native plugin arrives as a static
    /// library (`.a` / `.lib`) and is relinked into a shared library.
    #[serde(default)]
    pub link_args: Vec<String>,
    /// Passed to the plugin's `new` as JSON.
    #[serde(default = "empty_table")]
    pub config: toml::Value,
    /// Expected sha256 of the artifact (required for nothing, checked when set).
    pub sha256: Option<String>,
    /// Per-call timeout for isolated plugins.
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub limits: WasmLimits,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn empty_table() -> toml::Value {
    toml::Value::Table(Default::default())
}
fn yes() -> bool {
    true
}

impl PluginSpec {
    /// A spec with defaults for programmatic use.
    pub fn new(id: impl Into<String>, kind: PluginKind, source: Source) -> Self {
        PluginSpec {
            id: id.into(),
            kind,
            source,
            version: None,
            isolation: None,
            sandbox: SandboxPolicy::default(),
            engine: None,
            features: Vec::new(),
            no_default_features: false,
            link_args: Vec::new(),
            config: empty_table(),
            sha256: None,
            timeout_ms: None,
            limits: WasmLimits::default(),
            enabled: true,
        }
    }

    /// Isolation actually used for a native plugin.
    pub fn effective_isolation(&self) -> Isolation {
        self.isolation.unwrap_or_default()
    }

    pub fn config_json(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&self.config)?)
    }

    pub fn call_timeout(&self) -> Option<Duration> {
        self.timeout_ms.map(Duration::from_millis)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let ok = !self.id.is_empty()
            && self.id.len() <= 64
            && self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !ok {
            return Err(PluginError::Manifest(format!("plugin id `{}` must be 1-64 chars of [A-Za-z0-9_-]", self.id)));
        }
        if self.kind == PluginKind::Wasm && self.isolation.is_some() {
            return Err(PluginError::Manifest(format!(
                "plugin `{}`: wasm plugins are sandboxed by the engine; `isolation` applies to native only",
                self.id
            )));
        }
        if self.kind == PluginKind::Native && self.engine.is_some() {
            return Err(PluginError::Manifest(format!("plugin `{}`: `engine` only applies to wasm", self.id)));
        }
        if self.kind == PluginKind::Wasm && !self.link_args.is_empty() {
            return Err(PluginError::Manifest(format!("plugin `{}`: `link_args` only applies to native", self.id)));
        }
        // A typo here would otherwise surface on every load as a tampered artifact.
        if let Some(h) = &self.sha256
            && !(h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(PluginError::Manifest(format!(
                "plugin `{}`: sha256 must be 64 hex characters, got `{h}`",
                self.id
            )));
        }
        Ok(())
    }
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(io_err(path))?;
        let mut m = Self::parse(&text).map_err(|e| PluginError::Manifest(format!("{}: {e}", path.display())))?;
        m.base_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(m)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let m: Manifest = toml::from_str(text).map_err(|e| PluginError::Manifest(e.to_string()))?;
        m.settings.build.validate()?;
        let mut ids = std::collections::HashSet::new();
        for p in &m.plugins {
            p.validate()?;
            if !ids.insert(p.id.as_str()) {
                return Err(PluginError::Manifest(format!("duplicate plugin id `{}`", p.id)));
            }
        }
        Ok(m)
    }

    pub fn resolve(&self, p: &Path) -> PathBuf {
        if p.is_absolute() { p.to_owned() } else { self.base_dir.join(p) }
    }

    /// `plugwright.lock` next to the manifest.
    pub fn lock_path(&self) -> PathBuf {
        self.base_dir.join("plugwright.lock")
    }
}

/// `plugwright.lock`: resolved git SHAs, cache keys and artifact hashes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    #[serde(default, rename = "plugin")]
    pub plugins: Vec<LockEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockEntry {
    pub id: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
    pub sha256: String,
}

impl Lockfile {
    pub const VERSION: u32 = 1;

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let lock: Lockfile =
                    toml::from_str(&text).map_err(|e| PluginError::Manifest(format!("{}: {e}", path.display())))?;
                if lock.version != Self::VERSION {
                    return Err(PluginError::Manifest(format!(
                        "{}: unsupported lockfile version {}",
                        path.display(),
                        lock.version
                    )));
                }
                Ok(lock)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Lockfile { version: Self::VERSION, plugins: vec![] })
            }
            Err(e) => Err(PluginError::Io { path: path.to_owned(), source: e }),
        }
    }

    /// Write atomically (temp file + rename); a no-op if nothing changed on disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut sorted = self.clone();
        sorted.plugins.sort_by(|a, b| a.id.cmp(&b.id));
        let text = format!(
            "# Generated by plugwright. Pins git commits and artifact hashes.\n{}",
            toml::to_string(&sorted).map_err(|e| PluginError::Manifest(e.to_string()))?
        );
        if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
            return Ok(());
        }
        let tmp = path.with_extension("lock.tmp");
        std::fs::write(&tmp, text).map_err(io_err(&tmp))?;
        std::fs::rename(&tmp, path).map_err(io_err(path))
    }

    pub fn get(&self, id: &str) -> Option<&LockEntry> {
        self.plugins.iter().find(|e| e.id == id)
    }

    pub fn upsert(&mut self, entry: LockEntry) {
        match self.plugins.iter_mut().find(|e| e.id == entry.id) {
            Some(e) => *e = entry,
            None => self.plugins.push(entry),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.plugins.retain(|e| e.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_settings_and_link_args_parse_and_validate() {
        let m = Manifest::parse(
            r#"
            [settings.build]
            toolchain = "nightly"
            sccache = "off"
            linker = "lld"
            codegen = "auto"
            threads = 8
            profile = { opt_level = "s", codegen_units = 16, lto = "thin" }

            [[plugin]]
            id = "s"
            source = { artifact = "libs/libfoo.a" }
            link_args = ["-lz"]
            "#,
        )
        .unwrap();
        let b = &m.settings.build;
        assert_eq!(b.toolchain.as_deref(), Some("nightly"));
        assert_eq!(b.codegen, crate::accel::Codegen::Auto);
        assert_eq!(b.profile.codegen_units, Some(16));
        assert_eq!(m.plugins[0].link_args, ["-lz"]);

        let bad = |toml: &str| Manifest::parse(toml).is_err();
        assert!(bad("[settings.build]\nprofile = { lto = \"maybe\" }"));
        assert!(bad("[settings.build]\nprofile = { panic = \"abort\" }"), "panic must stay unwind");
        assert!(bad("[settings.build]\ncodegen = \"gcc\""));
        assert!(bad("[[plugin]]\nid = \"w\"\nkind = \"wasm\"\nsource = { path = \"x\" }\nlink_args = [\"-lz\"]"));
    }

    #[test]
    fn parses_all_source_kinds() {
        let m = Manifest::parse(
            r#"
            [settings]
            cache_dir = "c"
            locked = true

            [[plugin]]
            id = "a"
            source = { path = "native-hello" }
            version = "^0.1"
            isolation = "process"
            config = { greeting = "hi", n = 3 }

            [[plugin]]
            id = "b"
            kind = "wasm"
            engine = "wasmi"
            source = { git = "https://example.com/r.git", branch = "main", subdir = "p" }
            [plugin.limits]
            fuel = 1000
            timeout_ms = 50

            [[plugin]]
            id = "c"
            source = { artifact = "x.so" }
            sha256 = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
            [plugin.sandbox]
            network = true
            read = ["data"]
            "#,
        )
        .unwrap();
        assert!(m.settings.locked);
        assert_eq!(m.plugins[0].isolation, Some(Isolation::Process));
        assert_eq!(m.plugins[2].effective_isolation(), Isolation::Process, "process isolation is the default");
        assert!(m.plugins[0].sandbox.enabled && !m.plugins[0].sandbox.network);
        assert!(m.plugins[2].sandbox.network && m.plugins[2].sandbox.read == [PathBuf::from("data")]);
        assert_eq!(m.plugins[0].config_json().unwrap(), br#"{"greeting":"hi","n":3}"#);
        assert!(matches!(&m.plugins[1].source, Source::Git(g) if g.reference == GitRef::Branch("main".into())));
        assert_eq!(m.plugins[1].limits.fuel, Some(1000));
        assert_eq!(m.plugins[1].limits.timeout, Some(Duration::from_millis(50)));
        assert_eq!(m.plugins[2].source, Source::Artifact("x.so".into()));
    }

    #[test]
    fn rejects_bad_manifests() {
        let bad = [
            (
                r#"[[plugin]]
                id = "a"
                source = { path = "x", git = "https://e/r" }"#,
                "exactly one",
            ),
            (
                r#"[[plugin]]
                id = "a b"
                source = { path = "x" }"#,
                "must be",
            ),
            (
                r#"[[plugin]]
                id = "a"
                source = { path = "x" }
                typo = 1"#,
                "unknown field",
            ),
            (
                r#"[[plugin]]
                id = "a"
                kind = "wasm"
                isolation = "process"
                source = { path = "x" }"#,
                "native only",
            ),
            (
                r#"[[plugin]]
                id = "a"
                source = { path = "x" }
                [[plugin]]
                id = "a"
                source = { path = "y" }"#,
                "duplicate",
            ),
        ];
        for (text, needle) in bad {
            let err = Manifest::parse(text).unwrap_err().to_string();
            assert!(err.contains(needle), "{err:?} should mention {needle:?}");
        }
    }

    #[test]
    fn lockfile_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plugwright.lock");
        let mut l = Lockfile::load(&p).unwrap();
        l.upsert(LockEntry {
            id: "z".into(),
            source: "path".into(),
            git_sha: None,
            cache_key: Some("k".into()),
            sha256: "s".into(),
        });
        l.upsert(LockEntry {
            id: "a".into(),
            source: "git".into(),
            git_sha: Some("x".into()),
            cache_key: None,
            sha256: "t".into(),
        });
        l.save(&p).unwrap();
        let back = Lockfile::load(&p).unwrap();
        assert_eq!(back.plugins.len(), 2);
        assert_eq!(back.plugins[0].id, "a");
    }

    fn parse_err(text: &str) -> String {
        match Manifest::parse(text) {
            Err(PluginError::Manifest(m)) => m,
            other => panic!("expected a manifest error for {text:?}, got {other:?}"),
        }
    }

    fn plugin(extra: &str) -> String {
        format!("[[plugin]]\nid = \"p\"\nsource = {{ path = \"x\" }}\n{extra}")
    }

    fn entry(id: &str, sha256: &str) -> LockEntry {
        LockEntry { id: id.into(), source: "path+x".into(), git_sha: None, cache_key: None, sha256: sha256.into() }
    }

    #[test]
    fn empty_manifest_uses_secure_defaults() {
        let m = Manifest::parse("").unwrap();
        assert!(m.plugins.is_empty());
        let s = &m.settings;
        assert_eq!(s.cache_dir, Path::new(".plugwright"));
        assert!(s.release && !s.locked && !s.offline && !s.allow_local_git);
        assert_eq!((s.runner.as_ref(), s.max_restarts), (None, 3));

        let p = &Manifest::parse(&plugin("")).unwrap().plugins[0];
        assert_eq!(p.kind, PluginKind::Native);
        assert_eq!((p.isolation, p.effective_isolation()), (None, Isolation::Process));
        assert!(p.enabled && p.sandbox.enabled && !p.sandbox.network);
        assert!(p.features.is_empty() && !p.no_default_features && p.link_args.is_empty());
        assert_eq!((p.sha256.as_ref(), p.call_timeout()), (None, None));
    }

    #[test]
    fn plugin_ids_are_limited_in_length_and_charset() {
        let with_id = |id: &str| format!("[[plugin]]\nid = \"{id}\"\nsource = {{ path = \"x\" }}");
        let ok = format!("A-z_0{}", "9".repeat(59));
        assert_eq!(ok.len(), 64);
        assert_eq!(Manifest::parse(&with_id(&ok)).unwrap().plugins[0].id, ok);

        let too_long = "a".repeat(65);
        for bad in ["", "a b", "a/b", "../x", "a.b", "ü", too_long.as_str()] {
            let err = parse_err(&with_id(bad));
            assert!(err.contains(&format!("plugin id `{bad}`")) && err.contains("1-64 chars"), "{err}");
        }
    }

    #[test]
    fn sha256_pins_are_checked_for_format_when_parsed() {
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        for ok in [hash.to_owned(), hash.to_uppercase()] {
            let m = Manifest::parse(&plugin(&format!("sha256 = \"{ok}\""))).unwrap();
            assert_eq!(m.plugins[0].sha256.as_deref(), Some(ok.as_str()));
        }
        let long = format!("{hash}0");
        let prefixed = format!("sha256:{hash}");
        let spaced = format!(" {}", &hash[1..]);
        let non_hex = format!("g{}", &hash[1..]);
        for bad in ["", "00", &hash[..63], &long, &prefixed, &spaced, &non_hex] {
            let err = parse_err(&plugin(&format!("sha256 = \"{bad}\"")));
            assert_eq!(err, format!("plugin `p`: sha256 must be 64 hex characters, got `{bad}`"));
        }
    }

    #[test]
    fn duplicate_ids_name_the_offending_id() {
        let text = format!("{}\n{}", plugin(""), plugin(""));
        assert_eq!(parse_err(&text), "duplicate plugin id `p`");
    }

    #[test]
    fn unknown_fields_are_rejected_at_every_level() {
        let cases = [
            ("bogus = 1".to_owned(), "bogus"),
            ("[settings]\ncache_dirr = \"c\"".to_owned(), "cache_dirr"),
            ("[settings.build]\nlinkr = \"lld\"".to_owned(), "linkr"),
            ("[settings.build.profile]\npanic = \"abort\"".to_owned(), "panic"),
            ("[[plugin]]\nid = \"p\"\nsource = { path = \"x\", pth = \"y\" }".to_owned(), "pth"),
            (plugin("[plugin.limits]\nfuell = 1"), "fuell"),
            (plugin("[plugin.sandbox]\nnet = true"), "net"),
            (plugin("typo = 1"), "typo"),
        ];
        for (text, field) in cases {
            let err = parse_err(&text);
            assert!(err.contains("unknown field") && err.contains(&format!("`{field}`")), "{text:?}: {err}");
        }
    }

    #[test]
    fn kind_specific_fields_are_rejected_on_the_other_kind() {
        let wasm = |extra: &str| {
            format!("[[plugin]]\nid = \"w\"\nkind = \"wasm\"\nsource = {{ artifact = \"w.wasm\" }}\n{extra}")
        };
        assert_eq!(parse_err(&wasm("link_args = [\"-lz\"]")), "plugin `w`: `link_args` only applies to native");
        assert!(parse_err(&wasm("isolation = \"in-process\"")).contains("`isolation` applies to native only"));
        assert_eq!(parse_err(&plugin("engine = \"wasmi\"")), "plugin `p`: `engine` only applies to wasm");
        // Empty lists are the defaults, not a request.
        assert!(Manifest::parse(&wasm("link_args = []\nengine = \"wasmi\"")).is_ok());
    }

    #[test]
    fn source_tables_need_exactly_one_kind() {
        let src = |s: &str| format!("[[plugin]]\nid = \"p\"\nsource = {s}");
        assert!(parse_err(&src("{}")).contains("source needs one of `path`, `git`, `artifact`"));
        for s in [
            "{ path = \"x\", artifact = \"y\" }",
            "{ git = \"https://e/r\", artifact = \"y\" }",
            "{ path = \"x\", rev = \"abc\" }",
            "{ artifact = \"x\", subdir = \"s\" }",
        ] {
            assert!(parse_err(&src(s)).contains("exactly one of `path`, `git`, `artifact`"), "{s}");
        }
        for s in [
            "{ git = \"https://e/r\", rev = \"a\", tag = \"t\" }",
            "{ git = \"https://e/r\", branch = \"b\", tag = \"t\" }",
        ] {
            assert!(parse_err(&src(s)).contains("at most one of `rev`, `branch`, `tag`"), "{s}");
        }
    }

    #[test]
    fn git_source_without_a_reference_tracks_the_default_branch() {
        let m = Manifest::parse(
            "[[plugin]]\nid = \"a\"\nsource = { git = \"https://e/r\" }\n\
             [[plugin]]\nid = \"b\"\nsource = { git = \"https://e/r\", rev = \"0123abc\" }\n\
             [[plugin]]\nid = \"c\"\nsource = { git = \"https://e/r\", tag = \"v1\", subdir = \"crates/p\" }",
        )
        .unwrap();
        let git = |i: usize| match &m.plugins[i].source {
            Source::Git(g) => g.clone(),
            other => panic!("not git: {other:?}"),
        };
        assert_eq!(git(0), GitSource { url: "https://e/r".into(), reference: GitRef::DefaultBranch, subdir: None });
        assert_eq!(git(1).reference, GitRef::Rev("0123abc".into()));
        assert_eq!((git(2).reference, git(2).subdir), (GitRef::Tag("v1".into()), Some(PathBuf::from("crates/p"))));
    }

    #[test]
    fn timeouts_and_limits_parse_with_defaults_for_unset_limits() {
        let m = Manifest::parse(&plugin("timeout_ms = 250\n[plugin.limits]\nfuel = 7\nmax_memory_bytes = 1048576"))
            .unwrap();
        let p = &m.plugins[0];
        assert_eq!(p.call_timeout(), Some(Duration::from_millis(250)));
        assert_eq!((p.limits.fuel, p.limits.max_memory_bytes), (Some(7), Some(1 << 20)));
        let defaults = WasmLimits::default();
        assert_eq!(p.limits.timeout, defaults.timeout, "unset limits keep their defaults");
        assert_eq!(p.limits.max_stack_bytes, defaults.max_stack_bytes);

        assert!(parse_err(&plugin("timeout_ms = -1")).contains("timeout_ms"));
        assert!(parse_err(&plugin("timeout_ms = \"1s\"")).contains("timeout_ms"));
        assert!(parse_err(&plugin("[plugin.limits]\nfuel = -5")).contains("fuel"));
    }

    #[test]
    fn isolation_kind_and_version_values_are_checked() {
        let iso = |v: &str| Manifest::parse(&plugin(&format!("isolation = \"{v}\""))).map(|m| m.plugins[0].isolation);
        assert_eq!(iso("in-process").unwrap(), Some(Isolation::InProcess));
        assert_eq!(iso("process").unwrap(), Some(Isolation::Process));
        for bad in ["in_process", "thread", "Process"] {
            let err = iso(bad).unwrap_err().to_string();
            assert!(err.contains("unknown variant") && err.contains("in-process"), "{bad}: {err}");
        }
        assert!(parse_err(&plugin("kind = \"python\"")).contains("unknown variant `python`"));
        assert!(parse_err(&plugin("version = \"not a req\"")).contains("version"));
        let p = &Manifest::parse(&plugin("version = \">=1.2, <2\"\nenabled = false")).unwrap().plugins[0];
        assert!(p.version.as_ref().unwrap().matches(&semver::Version::new(1, 9, 0)) && !p.enabled);
    }

    #[test]
    fn build_settings_errors_name_the_bad_value() {
        let build = |body: &str| parse_err(&format!("[settings.build]\n{body}"));
        let cases = [
            ("profile = { lto = \"maybe\" }", "[settings.build] profile.lto `maybe` must be off, thin or fat"),
            (
                "profile = { strip = \"all\" }",
                "[settings.build] profile.strip `all` must be none, debuginfo or symbols",
            ),
            ("profile = { opt_level = \"4\" }", "[settings.build] profile.opt_level `4` must be 0-3, s or z"),
            ("profile = { debug = 3 }", "[settings.build] profile.debug must be 0-2"),
            ("profile = { codegen_units = 0 }", "[settings.build] profile.codegen_units must be at least 1"),
            ("threads = 0", "[settings.build] threads must be 1-256"),
            ("threads = 257", "[settings.build] threads must be 1-256"),
            ("toolchain = \"\"", "[settings.build] toolchain `` must be a rustup toolchain name"),
            ("toolchain = \"+nightly\"", "[settings.build] toolchain `+nightly` must be a rustup toolchain name"),
        ];
        for (body, want) in cases {
            assert_eq!(build(body), want, "{body}");
        }
        assert!(build("sccache = \"maybe\"").contains("unknown variant `maybe`"));
        assert!(build("linker = \"gold\"").contains("unknown variant `gold`"));
        // Whether the toolchain is nightly is only known when a build is planned.
        let m = Manifest::parse("[settings.build]\nthreads = 4\nprofile = { opt_level = \"3\", debug = 2 }").unwrap();
        assert_eq!(m.settings.build.threads, Some(4));
    }

    #[test]
    fn config_json_handles_absent_and_nested_config() {
        let m = Manifest::parse(&plugin("")).unwrap();
        assert_eq!(m.plugins[0].config_json().unwrap(), b"{}");
        assert_eq!(PluginSpec::new("x", PluginKind::Wasm, Source::Path("p".into())).config_json().unwrap(), b"{}");

        let m = Manifest::parse(&plugin(
            "[plugin.config]\nname = \"n\"\nlist = [1, 2.5, true]\n[plugin.config.db]\nport = 5432\nhosts = [\"a\", \"b\"]",
        ))
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&m.plugins[0].config_json().unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"name": "n", "list": [1, 2.5, true], "db": {"port": 5432, "hosts": ["a", "b"]}})
        );
    }

    #[test]
    fn load_resolves_paths_against_the_manifest_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugwright.toml");
        std::fs::write(&path, plugin("")).unwrap();
        let m = Manifest::load(&path).unwrap();
        assert_eq!(m.base_dir, dir.path());
        assert_eq!(m.lock_path(), dir.path().join("plugwright.lock"));
        assert_eq!(m.resolve(Path::new("a/b")), dir.path().join("a/b"));
        let abs = dir.path().join("abs");
        assert_eq!(m.resolve(&abs), abs);

        std::fs::write(&path, "[[plugin]]\nid = 1").unwrap();
        let err = Manifest::load(&path).unwrap_err().to_string();
        assert!(err.contains(&path.display().to_string()), "parse errors name the file: {err}");
        let missing = dir.path().join("missing.toml");
        assert!(matches!(Manifest::load(&missing), Err(PluginError::Io { path, .. }) if path == missing));
    }

    #[test]
    fn missing_lockfile_is_empty_and_bad_lockfiles_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plugwright.lock");
        assert_eq!(Lockfile::load(&p).unwrap(), Lockfile { version: Lockfile::VERSION, plugins: vec![] });

        std::fs::write(&p, "this is = = not toml").unwrap();
        let err = Lockfile::load(&p).unwrap_err().to_string();
        assert!(err.starts_with("manifest error: ") && err.contains(&p.display().to_string()), "{err}");

        std::fs::write(&p, "version = 2\n").unwrap();
        assert!(Lockfile::load(&p).unwrap_err().to_string().contains("unsupported lockfile version 2"));

        std::fs::write(&p, "version = 1\n[[plugin]]\nid = \"a\"\nsource = \"s\"\n").unwrap();
        assert!(Lockfile::load(&p).unwrap_err().to_string().contains("sha256"), "entries need a hash");

        // A directory where the lockfile should be is an I/O error, not an empty lock.
        let d = dir.path().join("dir.lock");
        std::fs::create_dir(&d).unwrap();
        assert!(matches!(Lockfile::load(&d), Err(PluginError::Io { .. })));
    }

    #[test]
    fn lockfile_save_round_trips_sorted_and_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plugwright.lock");
        let mut l = Lockfile { version: Lockfile::VERSION, plugins: vec![] };
        l.upsert(entry("b", "1"));
        l.upsert(LockEntry { git_sha: Some("f".repeat(40)), cache_key: Some("k".into()), ..entry("a", "2") });
        l.save(&p).unwrap();

        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# Generated by plugwright."), "{text}");
        assert!(!text.contains("git_sha = \"\"") && text.matches("git_sha").count() == 1, "None is omitted: {text}");
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["plugwright.lock"], "no temp file is left behind");

        let back = Lockfile::load(&p).unwrap();
        assert_eq!(back.plugins.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(back.get("a").unwrap().git_sha.as_deref(), Some("f".repeat(40).as_str()));
        assert_eq!(back.get("b"), Some(&entry("b", "1")));
        assert_eq!(l.plugins[0].id, "b", "saving does not reorder the in-memory lock");
    }

    #[test]
    fn lockfile_save_skips_unchanged_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plugwright.lock");
        let mut l = Lockfile { version: Lockfile::VERSION, plugins: vec![entry("a", "1")] };
        l.save(&p).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
        l.save(&p).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), old, "identical content is not rewritten");

        l.upsert(entry("a", "2"));
        l.save(&p).unwrap();
        assert_ne!(std::fs::metadata(&p).unwrap().modified().unwrap(), old);
        assert_eq!(Lockfile::load(&p).unwrap().get("a").unwrap().sha256, "2");
    }

    #[test]
    fn lockfile_upsert_replaces_and_remove_deletes_by_id() {
        let mut l = Lockfile::default();
        l.upsert(entry("a", "1"));
        l.upsert(entry("b", "1"));
        l.upsert(entry("a", "2"));
        assert_eq!(l.plugins.len(), 2);
        assert_eq!(l.get("a").unwrap().sha256, "2");
        l.remove("a");
        l.remove("missing");
        assert_eq!((l.get("a"), l.plugins.len()), (None, 1));
    }
}
