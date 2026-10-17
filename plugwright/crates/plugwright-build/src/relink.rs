//! Turn a static library (`.a`, or `.lib` on MSVC) into a shared library the
//! native loader can open, cached like any build.
//!
//! The linker is told that the plugin entry symbol is undefined (`-u` /
//! `/INCLUDE`), so it pulls exactly the archive members the plugin needs, and
//! that symbol is the only export (version script / `-exported_symbol` /
//! `/EXPORT`). `--whole-archive` was rejected: it drags in every object of a
//! Rust staticlib (all of std) and can collide with the C runtime's symbols.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use plugwright_abi::NATIVE_ENTRY;
use plugwright_core::error::{PluginError, Result, io_err};

use crate::builder::{Builder, rustc_version};
use crate::cache::{Artifact, CacheEntry, sha256_hex};

/// Bump when the link recipe changes.
const RELINK_SCHEMA: &str = "plugwright-relink-v1";

/// A static library to relink.
#[derive(Debug, Clone)]
pub struct RelinkRequest {
    pub id: String,
    pub archive: PathBuf,
    /// sha256 of `archive` (already computed by the caller).
    pub archive_sha256: String,
    /// Extra linker arguments, e.g. `-lssl` or `ws2_32.lib`, for archives
    /// that need more than the Rust standard library's native libraries.
    pub link_args: Vec<String>,
}

/// How to drive the platform linker.
#[derive(Debug, PartialEq, Eq)]
enum Flavor {
    /// `cc` on Linux and other ELF systems.
    Elf,
    /// `cc` on Apple platforms (ld64 symbol names carry a `_` prefix).
    Apple,
    /// `link.exe`.
    Msvc,
}

impl Flavor {
    /// `None` for `*-windows-gnu*`, where relinking is unsupported.
    fn of(triple: &str) -> Option<Flavor> {
        if triple.contains("-apple-") {
            Some(Flavor::Apple)
        } else if triple.ends_with("-windows-msvc") {
            Some(Flavor::Msvc)
        } else if !triple.contains("-windows") {
            Some(Flavor::Elf)
        } else {
            None
        }
    }
}

impl Builder {
    /// Link `req.archive` into a shared library, or return the cached result.
    pub fn relink(&self, req: &RelinkRequest) -> Result<Artifact> {
        let _span = tracing::info_span!("relink", plugin = %req.id).entered();
        let fail = |message: String| PluginError::build(&req.id, message);
        let dir = req.archive.parent().unwrap_or(Path::new("."));
        // The host triple does not depend on where the archive lives; asking in
        // the cache root lets every relink share one remembered `rustc -vV`.
        let triple = triple_of(&self.rustc_vv(self.cache().root(), None)?)?;
        let Some(flavor) = Flavor::of(&triple) else {
            return Err(PluginError::Unsupported(format!("relinking static libraries on {triple}")));
        };
        let linker = self.linker(&triple, &flavor).map_err(|e| fail(format!("no linker for {triple}: {e}")))?;
        let libs = self.native_static_libs(dir, &triple);

        let mut key = format!("{RELINK_SCHEMA}\n{}\n{triple}\n{:?}\n", req.archive_sha256, linker.get_program());
        for a in
            linker.get_args().chain(libs.iter().map(|s| s.as_ref())).chain(req.link_args.iter().map(|s| s.as_ref()))
        {
            key.push_str(&a.to_string_lossy());
            key.push('\n');
        }
        let key = sha256_hex(key.as_bytes());
        if let Some(a) = self.cache().lookup(&key)? {
            tracing::info!(key = &key[..12], "relinked library cache hit");
            return Ok(a);
        }
        let _lock = self.cache().lock()?;
        if let Some(a) = self.cache().lookup(&key)? {
            return Ok(a);
        }

        let work = self.cache().root().join("meta").join(format!("relink-{}-{}", std::process::id(), &key[..16]));
        std::fs::create_dir_all(&work).map_err(io_err(&work))?;
        let ext = match flavor {
            Flavor::Elf => "so",
            Flavor::Apple => "dylib",
            Flavor::Msvc => "dll",
        };
        let out = work.join(format!("plugin.{ext}"));
        let mut cmd = linker;
        match flavor {
            Flavor::Elf => {
                let map = work.join("exports.map");
                std::fs::write(&map, format!("{{ global: {NATIVE_ENTRY}; local: *; }};\n")).map_err(io_err(&map))?;
                cmd.arg("-shared").arg("-o").arg(&out).arg(format!("-Wl,-u,{NATIVE_ENTRY}"));
                cmd.arg(format!("-Wl,--version-script={}", map.display()));
                cmd.arg(&req.archive);
            }
            Flavor::Apple => {
                cmd.arg("-dynamiclib").arg("-o").arg(&out);
                cmd.arg(format!("-Wl,-u,_{NATIVE_ENTRY}")).arg(format!("-Wl,-exported_symbol,_{NATIVE_ENTRY}"));
                cmd.arg(&req.archive);
            }
            Flavor::Msvc => {
                cmd.args(["/NOLOGO", "/DLL"]).arg(format!("/OUT:{}", out.display()));
                cmd.arg(format!("/INCLUDE:{NATIVE_ENTRY}")).arg(format!("/EXPORT:{NATIVE_ENTRY}"));
                cmd.arg(&req.archive);
            }
        }
        cmd.args(&libs).args(&req.link_args).stdin(Stdio::null());
        tracing::debug!(?cmd, "linking");
        let output = cmd.output().map_err(|e| fail(format!("cannot run linker {:?}: {e}", cmd.get_program())));
        let result = output.and_then(|o| {
            if o.status.success() && out.is_file() {
                Ok(())
            } else {
                let mut text = String::from_utf8_lossy(&o.stderr).into_owned();
                text.push_str(&String::from_utf8_lossy(&o.stdout));
                Err(fail(format!("linking the static library failed ({}):\n{}", o.status, text.trim())))
            }
        });
        let stored = result.and_then(|()| {
            let entry = CacheEntry {
                key: key.clone(),
                id: req.id.clone(),
                file: String::new(),
                sha256: String::new(),
                size: 0,
                source: format!("relink+{}", req.archive.display()),
                target: triple.clone(),
                built_at_unix: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            };
            self.cache().store(&key, &out, entry)
        });
        let _ = std::fs::remove_dir_all(&work);
        let a = stored?;
        tracing::info!(key = &key[..12], "static library relinked into a shared library");
        Ok(a)
    }

    /// Native libraries the Rust standard library needs in a staticlib,
    /// as reported by `rustc --print native-static-libs` (memoised per triple).
    /// [`linker`], remembered per process and in `meta/`: `cc`'s discovery runs
    /// `xcrun` and probes the compiler (~0.6 s on macOS), and its answer depends
    /// only on the triple and the environment variables it reads.
    fn linker(&self, triple: &str, flavor: &Flavor) -> std::result::Result<Command, String> {
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock};
        static MEMO: OnceLock<Mutex<HashMap<String, LinkerCmd>>> = OnceLock::new();
        let fp = linker_fingerprint(triple);
        let memo = MEMO.get_or_init(Default::default);
        if let Some(c) = memo.lock().unwrap_or_else(|p| p.into_inner()).get(&fp) {
            return Ok(c.command());
        }
        let file = self.cache().root().join("meta").join(format!("linker-{}.json", &fp[..16]));
        match remembered_linker(&file, &fp, || linker(triple, flavor))? {
            Ok(cmd) => {
                let out = cmd.command();
                memo.lock().unwrap_or_else(|p| p.into_inner()).insert(fp, cmd);
                Ok(out)
            }
            // A non-UTF-8 command cannot be stored; it is resolved every time.
            Err(cmd) => Ok(cmd),
        }
    }

    fn native_static_libs(&self, dir: &Path, triple: &str) -> Vec<String> {
        let memo = self.cache().root().join("meta").join(format!("native-static-libs-{triple}.txt"));
        if let Ok(s) = std::fs::read_to_string(&memo) {
            return s.split_whitespace().map(str::to_owned).collect();
        }
        let probe = self.cache().root().join("meta").join(format!("nsl-probe-{}", std::process::id()));
        let libs = probe_native_libs(dir, &probe).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "cannot ask rustc for native-static-libs; using defaults");
            default_native_libs(triple)
        });
        let _ = std::fs::remove_dir_all(&probe);
        let _ = std::fs::write(&memo, libs.join(" "));
        libs
    }
}

/// The linker static libraries would be relinked with on this host, for diagnostics.
pub fn host_linker(dir: &Path) -> std::result::Result<String, String> {
    let triple = host_triple(dir).map_err(|e| e.to_string())?;
    let Some(flavor) = Flavor::of(&triple) else {
        return Err(format!("relinking is not supported on {triple}"));
    };
    linker(&triple, &flavor).map(|c| c.get_program().to_string_lossy().into_owned())
}

fn host_triple(dir: &Path) -> Result<String> {
    triple_of(&rustc_version(dir, None)?)
}

fn triple_of(rustc_vv: &str) -> Result<String> {
    rustc_vv
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(|t| t.trim().to_owned())
        .ok_or_else(|| PluginError::build("", "rustc -vV printed no host triple"))
}

/// A resolved linker command in a form that can be stored.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LinkerCmd {
    fingerprint: String,
    program: String,
    args: Vec<String>,
    /// `None` removes the variable.
    env: Vec<(String, Option<String>)>,
}

impl LinkerCmd {
    fn of(cmd: &Command, fingerprint: String) -> Option<Self> {
        let s = |o: &std::ffi::OsStr| o.to_str().map(str::to_owned);
        Some(LinkerCmd {
            fingerprint,
            program: s(cmd.get_program())?,
            args: cmd.get_args().map(s).collect::<Option<_>>()?,
            env: cmd
                .get_envs()
                .map(|(k, v)| {
                    Some((
                        s(k)?,
                        match v {
                            Some(v) => Some(s(v)?),
                            None => None,
                        },
                    ))
                })
                .collect::<Option<_>>()?,
        })
    }

    fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args);
        for (k, v) in &self.env {
            match v {
                Some(v) => c.env(k, v),
                None => c.env_remove(k),
            };
        }
        c
    }

    /// A remembered absolute path that has disappeared (toolchain removed) is re-resolved.
    fn still_there(&self) -> bool {
        let p = Path::new(&self.program);
        !p.is_absolute() || p.is_file()
    }
}

/// The linker stored in `file` for fingerprint `fp`, or `resolve`d and stored.
/// `Err` carries a command that cannot be stored (not UTF-8).
fn remembered_linker(
    file: &Path,
    fp: &str,
    resolve: impl FnOnce() -> std::result::Result<Command, String>,
) -> std::result::Result<std::result::Result<LinkerCmd, Command>, String> {
    let stored = std::fs::read(file)
        .ok()
        .and_then(|raw| serde_json::from_slice::<LinkerCmd>(&raw).ok())
        .filter(|c| c.fingerprint == fp && c.still_there());
    if let Some(c) = stored {
        return Ok(Ok(c));
    }
    let cmd = resolve()?;
    let Some(c) = LinkerCmd::of(&cmd, fp.to_owned()) else { return Ok(Err(cmd)) };
    if let Ok(raw) = serde_json::to_vec(&c) {
        let _ = std::fs::write(file, raw);
    }
    Ok(Ok(c))
}

/// The triple plus every environment variable `cc` consults when it picks a
/// compiler (`CC*`, `CFLAGS*`, `TARGET_*`, `HOST_*`, `CRATE_CC_*`, `PATH`,
/// Apple SDK and deployment variables, Visual Studio variables).
fn linker_fingerprint(triple: &str) -> String {
    const PREFIXES: &[&str] = &["CC", "CFLAGS", "TARGET_", "HOST_", "CRATE_CC", "VS", "WINDOWSSDK", "UCRT"];
    const EXACT: &[&str] = &["PATH", "SDKROOT", "DEVELOPER_DIR", "PLATFORM", "LIB", "LIBPATH", "INCLUDE"];
    let mut vars: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| {
            let k = k.to_string_lossy().into_owned();
            let up = k.to_ascii_uppercase();
            let keep = PREFIXES.iter().any(|p| up.starts_with(p))
                || EXACT.contains(&up.as_str())
                || up.ends_with("_DEPLOYMENT_TARGET")
                || up.ends_with("INSTALLDIR")
                || up.starts_with("VISUALSTUDIO");
            keep.then(|| (k, v.to_string_lossy().into_owned()))
        })
        .collect();
    vars.sort();
    let mut text = format!("{RELINK_SCHEMA}\n{triple}\n");
    for (k, v) in vars {
        text.push_str(&k);
        text.push('=');
        text.push_str(&v);
        text.push('\n');
    }
    sha256_hex(text.as_bytes())
}

/// The C compiler driver (Unix) or `link.exe` (MSVC), found the way build
/// scripts find them (`CC`, `PATH`, Visual Studio discovery).
fn linker(triple: &str, flavor: &Flavor) -> std::result::Result<Command, String> {
    if let Flavor::Msvc = flavor {
        return cc::windows_registry::find(triple, "link.exe").ok_or_else(|| "link.exe not found".into());
    }
    let tool = cc::Build::new()
        .target(triple)
        .host(triple)
        .opt_level(0)
        .debug(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .try_get_compiler()
        .map_err(|e| e.to_string())?;
    Ok(tool.to_command())
}

fn probe_native_libs(dir: &Path, probe: &Path) -> std::result::Result<Vec<String>, String> {
    // rustc runs in `dir`, so a relative cache root would resolve against the wrong directory.
    let probe = &std::path::absolute(probe).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(probe).map_err(|e| e.to_string())?;
    let src = probe.join("probe.rs");
    std::fs::write(&src, "").map_err(|e| e.to_string())?;
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let out = Command::new(rustc)
        .current_dir(dir)
        .args(["--print", "native-static-libs", "--crate-type", "staticlib", "--crate-name", "pw_probe"])
        .arg("-o")
        .arg(probe.join("probe.a"))
        .arg(&src)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines()
        .find_map(|l| l.split_once("native-static-libs:").map(|(_, libs)| libs))
        .map(|libs| libs.split_whitespace().map(str::to_owned).collect())
        .ok_or_else(|| {
            let detail = text.lines().find(|l| l.starts_with("error")).unwrap_or_default();
            format!("rustc did not print native-static-libs ({}) {detail}", out.status)
        })
}

/// Used when `rustc` cannot be asked (values from rustc 1.97).
fn default_native_libs(triple: &str) -> Vec<String> {
    let libs: &[&str] = if triple.contains("-apple-") {
        &["-lSystem", "-lc", "-lm"]
    } else if triple.ends_with("-windows-msvc") {
        &["kernel32.lib", "advapi32.lib", "ntdll.lib", "userenv.lib", "ws2_32.lib", "dbghelp.lib", "/defaultlib:msvcrt"]
    } else {
        &["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl", "-lc"]
    };
    libs.iter().map(|s| (*s).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::BuildOptions;
    use crate::cache::Cache;

    fn builder(root: &Path) -> Builder {
        Builder::new(Cache::new(root).unwrap(), BuildOptions::default())
    }

    #[test]
    fn linker_is_resolved_once_and_then_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("linker.json");
        let resolve = || {
            let mut c = Command::new("cc");
            c.arg("-m64").env("SDKROOT", "/sdk").env_remove("CFLAGS");
            Ok(c)
        };
        let first = remembered_linker(&file, "fp", resolve).unwrap().unwrap();
        assert_eq!((first.program.as_str(), first.args.as_slice()), ("cc", ["-m64".to_owned()].as_slice()));
        let stored = remembered_linker(&file, "fp", || Err("must not resolve again".into())).unwrap().unwrap();
        let cmd = stored.command();
        assert_eq!(cmd.get_program(), "cc");
        let envs: Vec<_> = cmd.get_envs().map(|(k, v)| (k.to_owned(), v.map(|v| v.to_owned()))).collect();
        assert_eq!(envs.len(), 2, "set and removed variables survive: {envs:?}");

        // Another environment, or a linker that is gone, is resolved again.
        assert!(remembered_linker(&file, "other", || Err("resolved".into())).is_err());
        let mut gone = stored.clone();
        gone.program = dir.path().join("missing-cc").display().to_string();
        std::fs::write(&file, serde_json::to_vec(&gone).unwrap()).unwrap();
        assert!(remembered_linker(&file, "fp", || Err("resolved".into())).is_err());
        std::fs::write(&file, b"{").unwrap();
        assert!(remembered_linker(&file, "fp", resolve).unwrap().is_ok(), "a corrupt memo is replaced");
    }

    #[test]
    fn linker_fingerprint_follows_the_triple() {
        assert_eq!(linker_fingerprint("a-b-c"), linker_fingerprint("a-b-c"));
        assert_ne!(linker_fingerprint("a-b-c"), linker_fingerprint("x-y-z"));
    }

    #[test]
    fn linker_flavor_follows_the_triple() {
        for (triple, flavor) in [
            ("aarch64-apple-darwin", Some(Flavor::Apple)),
            ("x86_64-apple-ios", Some(Flavor::Apple)),
            ("x86_64-pc-windows-msvc", Some(Flavor::Msvc)),
            ("aarch64-pc-windows-msvc", Some(Flavor::Msvc)),
            ("x86_64-unknown-linux-gnu", Some(Flavor::Elf)),
            ("aarch64-unknown-linux-musl", Some(Flavor::Elf)),
            ("x86_64-unknown-freebsd", Some(Flavor::Elf)),
            ("x86_64-pc-windows-gnu", None),
            ("x86_64-pc-windows-gnullvm", None),
        ] {
            assert_eq!(Flavor::of(triple), flavor, "{triple}");
        }
    }

    #[test]
    fn default_native_libs_match_the_platform_linker() {
        assert_eq!(default_native_libs("aarch64-apple-darwin"), ["-lSystem", "-lc", "-lm"]);
        let msvc = default_native_libs("x86_64-pc-windows-msvc");
        assert!(msvc.iter().all(|l| l.ends_with(".lib") || l.starts_with("/defaultlib:")), "{msvc:?}");
        assert!(msvc.contains(&"ntdll.lib".to_owned()));
        let elf = default_native_libs("x86_64-unknown-linux-gnu");
        assert!(elf.iter().all(|l| l.starts_with("-l")), "{elf:?}");
        assert_eq!(elf.last().map(String::as_str), Some("-lc"));
    }

    #[test]
    fn native_static_libs_are_memoised_per_triple() {
        let dir = tempfile::tempdir().unwrap();
        let b = builder(dir.path());
        std::fs::write(dir.path().join("meta/native-static-libs-fake-triple.txt"), "-lfoo  -lbar\n").unwrap();
        assert_eq!(b.native_static_libs(dir.path(), "fake-triple"), ["-lfoo", "-lbar"]);
    }

    #[test]
    fn native_static_libs_fall_back_to_defaults_and_remember_them() {
        let dir = tempfile::tempdir().unwrap();
        let b = builder(&dir.path().join("cache"));
        // rustc cannot even start in a directory that does not exist.
        let triple = "x86_64-unknown-linux-gnu";
        let libs = b.native_static_libs(&dir.path().join("missing"), triple);
        assert_eq!(libs, default_native_libs(triple));
        let memo = std::fs::read_to_string(dir.path().join(format!("cache/meta/native-static-libs-{triple}.txt")));
        assert_eq!(memo.unwrap(), libs.join(" "));
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("cache/meta"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("nsl-probe-"))
            .collect();
        assert!(leftovers.is_empty(), "the probe dir is removed: {leftovers:?}");
    }

    /// Regression: rustc runs in `dir`, so a relative probe path (from a
    /// relative cache root) used to resolve against the wrong directory.
    #[cfg(unix)]
    #[test]
    fn probe_works_with_a_relative_probe_dir_and_a_different_working_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = std::env::current_dir().unwrap();
        // `tmp/probe` spelled relative to this process's working directory.
        let up = std::iter::repeat_n("..", cwd.components().count() - 1).collect::<PathBuf>();
        let rel = up.join(tmp.path().strip_prefix("/").unwrap()).join("probe");
        assert!(rel.is_relative());
        // Deeper than the cwd, so `rel` points nowhere when resolved from here.
        let elsewhere = tmp.path().join("a/b/c/d/e/f/g/h/i/j/k/l");
        std::fs::create_dir_all(&elsewhere).unwrap();

        let libs = probe_native_libs(&elsewhere, &rel).unwrap();
        assert!(!libs.is_empty());
        assert!(tmp.path().join("probe/probe.rs").is_file());
        assert!(!elsewhere.join(&rel).exists());
    }
}
