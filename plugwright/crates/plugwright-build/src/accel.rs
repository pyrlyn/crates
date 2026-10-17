//! Build acceleration (`[settings.build]`): a compiler cache (sccache), a
//! faster linker (mold / lld), the Cranelift backend and the parallel front
//! end on nightly, and profile tuning.
//!
//! Everything that can change the produced bytes ends up in the cache key
//! ([`Plan::key_material`]); the compiler cache does not, since it only
//! replays what rustc would have produced.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use plugwright_core::error::{PluginError, Result};

use crate::builder::Target;

/// `auto` uses a tool when it is installed, `on` requires it, `off` never uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Toggle {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkerChoice {
    /// mold on Linux when installed; otherwise the toolchain default
    /// (rust-lld on x86_64 Linux, ld-prime on macOS, link.exe on MSVC).
    #[default]
    Auto,
    /// Whatever rustc picks.
    Default,
    /// mold (Linux).
    Mold,
    /// LLVM lld: `ld.lld` (Linux), `ld64.lld` (macOS), `lld-link` (MSVC).
    Lld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Codegen {
    #[default]
    Llvm,
    /// Cranelift on nightly (`rustup component add rustc-codegen-cranelift-preview`).
    Cranelift,
    /// Cranelift for dev builds when the toolchain has it, LLVM otherwise.
    Auto,
}

/// Overrides for the cargo profile plugins are built with (`--config profile.*`).
/// `panic` is deliberately absent: plugins must unwind.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileTuning {
    /// `0`-`3`, `s` or `z`.
    pub opt_level: Option<String>,
    pub codegen_units: Option<u32>,
    /// `off`, `thin` or `fat`.
    pub lto: Option<String>,
    /// Debug info level `0`-`2`.
    pub debug: Option<u8>,
    /// `none`, `debuginfo` or `symbols`.
    pub strip: Option<String>,
    pub incremental: Option<bool>,
}

/// `[settings.build]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Accel {
    /// rustup toolchain for plugin builds (`cargo +<toolchain>`), e.g. `nightly`.
    pub toolchain: Option<String>,
    pub sccache: Toggle,
    pub linker: LinkerChoice,
    pub codegen: Codegen,
    /// rustc front-end threads (`-Zthreads`), nightly only.
    pub threads: Option<u32>,
    pub profile: ProfileTuning,
}

/// What `RUSTC_WRAPPER` the cargo child gets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Wrapper {
    /// Leave the environment and cargo config alone.
    #[default]
    Keep,
    Set(PathBuf),
    /// An empty `RUSTC_WRAPPER` also overrides `build.rustc-wrapper`.
    Disable,
}

/// Accelerators resolved for one build.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub wrapper: Wrapper,
    pub rustflags: Vec<String>,
    /// Extra cargo arguments (`-Z…`, `--config profile.…`).
    pub cargo_args: Vec<String>,
    /// Human-readable list of what is applied, for logs and `doctor`.
    pub applied: Vec<String>,
}

impl Plan {
    /// The part of the plan that can change the artifact's bytes.
    pub fn key_material(&self) -> String {
        let mut s = self.rustflags.join("\x1f");
        s.push('\n');
        s.push_str(&self.cargo_args.join("\x1f"));
        s
    }

    /// Apply the plan to a `cargo build` command.
    pub fn apply(&self, cmd: &mut Command) {
        match &self.wrapper {
            Wrapper::Keep => {}
            Wrapper::Set(p) => {
                cmd.env("RUSTC_WRAPPER", p);
            }
            Wrapper::Disable => {
                cmd.env("RUSTC_WRAPPER", "");
            }
        }
        cmd.args(&self.cargo_args);
        if self.rustflags.is_empty() {
            return;
        }
        // Environment flags replace config flags wholesale, so extend
        // whichever source cargo would actually use.
        if let Some(enc) = std::env::var_os("CARGO_ENCODED_RUSTFLAGS") {
            let mut v = enc.to_string_lossy().into_owned();
            for f in &self.rustflags {
                if !v.is_empty() {
                    v.push('\x1f');
                }
                v.push_str(f);
            }
            cmd.env("CARGO_ENCODED_RUSTFLAGS", v);
        } else if let Ok(env) = std::env::var("RUSTFLAGS") {
            let all: Vec<&str> = env.split_whitespace().chain(self.rustflags.iter().map(String::as_str)).collect();
            cmd.env("CARGO_ENCODED_RUSTFLAGS", all.join("\x1f")).env_remove("RUSTFLAGS");
        } else {
            // `--config` arrays are appended to `build.rustflags` from config files.
            cmd.arg("--config").arg(format!("build.rustflags={}", toml_array(&self.rustflags)));
        }
    }
}

/// What the build needs that the accelerators must respect.
#[derive(Debug, Clone, Copy)]
pub struct PlanInput<'a> {
    /// `rustc -vV` of the toolchain that will build the plugin.
    pub rustc_vv: &'a str,
    /// rustup toolchain override (`Accel::toolchain`).
    pub toolchain: Option<&'a str>,
    /// Directory to run probes in (toolchain files apply).
    pub dir: &'a Path,
    pub release: bool,
    pub target: Target,
    /// The artifact will be loaded into the host: panics must unwind.
    pub needs_unwind: bool,
}

impl Accel {
    /// Check values that serde cannot.
    pub fn validate(&self) -> Result<()> {
        let bad = |what: String| Err(PluginError::Manifest(format!("[settings.build] {what}")));
        if let Some(t) = &self.toolchain
            && (t.is_empty() || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
        {
            return bad(format!("toolchain `{t}` must be a rustup toolchain name"));
        }
        if self.threads.is_some_and(|n| !(1..=256).contains(&n)) {
            return bad("threads must be 1-256".into());
        }
        let p = &self.profile;
        if let Some(o) = &p.opt_level
            && !["0", "1", "2", "3", "s", "z"].contains(&o.as_str())
        {
            return bad(format!("profile.opt_level `{o}` must be 0-3, s or z"));
        }
        if let Some(l) = &p.lto
            && !["off", "thin", "fat"].contains(&l.as_str())
        {
            return bad(format!("profile.lto `{l}` must be off, thin or fat"));
        }
        if let Some(s) = &p.strip
            && !["none", "debuginfo", "symbols"].contains(&s.as_str())
        {
            return bad(format!("profile.strip `{s}` must be none, debuginfo or symbols"));
        }
        if p.debug.is_some_and(|d| d > 2) {
            return bad("profile.debug must be 0-2".into());
        }
        if p.codegen_units == Some(0) {
            return bad("profile.codegen_units must be at least 1".into());
        }
        Ok(())
    }

    /// Resolve the accelerators for one build. Explicitly requested tools that
    /// are missing are errors; `auto` ones are skipped.
    pub fn plan(&self, input: PlanInput<'_>) -> std::result::Result<Plan, String> {
        self.validate().map_err(|e| e.to_string())?;
        let mut plan = Plan::default();
        let nightly = is_nightly(input.rustc_vv);
        let native = input.target == Target::Native;

        plan.wrapper = match self.sccache {
            Toggle::Off => Wrapper::Disable,
            _ if std::env::var_os("RUSTC_WRAPPER").is_some_and(|w| !w.is_empty()) => Wrapper::Keep,
            Toggle::Auto => find_tool("sccache").map_or(Wrapper::Keep, Wrapper::Set),
            Toggle::On => Wrapper::Set(find_tool("sccache").ok_or("sccache = \"on\" but sccache is not on PATH")?),
        };
        if let Wrapper::Set(p) = &plan.wrapper {
            plan.applied.push(format!("sccache ({})", p.display()));
        }

        if native {
            let linker = match self.linker {
                LinkerChoice::Default => None,
                LinkerChoice::Auto => cfg!(target_os = "linux").then(|| find_tool("mold").map(|_| "mold")).flatten(),
                LinkerChoice::Mold if cfg!(target_os = "linux") => {
                    find_tool("mold").ok_or("linker = \"mold\" but mold is not on PATH")?;
                    Some("mold")
                }
                LinkerChoice::Mold => return Err("linker = \"mold\" is supported on Linux only".into()),
                LinkerChoice::Lld => {
                    let tool = lld_name();
                    find_tool(tool).ok_or_else(|| format!("linker = \"lld\" but {tool} is not on PATH"))?;
                    Some("lld")
                }
            };
            match linker {
                Some(_) if cfg!(windows) => plan.rustflags.push(format!("-Clinker={}", lld_name())),
                Some(l) => plan.rustflags.push(format!("-Clink-arg=-fuse-ld={l}")),
                None => {}
            }
            if let Some(l) = linker {
                plan.applied.push(format!("linker {l}"));
            }
        }

        let want_clif = match self.codegen {
            Codegen::Llvm => false,
            Codegen::Auto => !input.release && nightly && has_cranelift(input),
            Codegen::Cranelift => {
                if !nightly {
                    return Err(
                        "codegen = \"cranelift\" needs a nightly toolchain (set toolchain = \"nightly\")".into()
                    );
                }
                if !has_cranelift(input) {
                    return Err("codegen = \"cranelift\" but the toolchain lacks it; run \
                                `rustup component add rustc-codegen-cranelift-preview --toolchain nightly`"
                        .into());
                }
                true
            }
        };
        if want_clif {
            if !native {
                plan.applied.push("llvm (cranelift does not target wasm)".into());
            } else if input.needs_unwind {
                // Cranelift-built code aborts on panic instead of unwinding;
                // in the host process that would take the host down.
                plan.applied.push("llvm (cranelift does not unwind; in-process plugins need it)".into());
            } else {
                let profile = if input.release { "release" } else { "dev" };
                plan.cargo_args.push("-Zcodegen-backend".into());
                plan.cargo_args.push("--config".into());
                plan.cargo_args.push(format!("profile.{profile}.codegen-backend=\"cranelift\""));
                plan.applied.push("cranelift".into());
            }
        }

        if let Some(n) = self.threads {
            if !nightly {
                return Err("threads needs a nightly toolchain (-Zthreads)".into());
            }
            plan.rustflags.push(format!("-Zthreads={n}"));
            plan.applied.push(format!("{n} front-end threads"));
        }

        let profile = if input.release { "release" } else { "dev" };
        let p = &self.profile;
        let mut set = |k: &str, v: String| {
            plan.cargo_args.push("--config".into());
            plan.cargo_args.push(format!("profile.{profile}.{k}={v}"));
            plan.applied.push(format!("{k}={v}"));
        };
        if let Some(o) = &p.opt_level {
            set("opt-level", if o.parse::<u8>().is_ok() { o.clone() } else { format!("\"{o}\"") });
        }
        if let Some(n) = p.codegen_units {
            set("codegen-units", n.to_string());
        }
        if let Some(l) = &p.lto {
            set("lto", format!("\"{l}\""));
        }
        if let Some(d) = p.debug {
            set("debug", d.to_string());
        }
        if let Some(s) = &p.strip {
            set("strip", format!("\"{s}\""));
        }
        if let Some(i) = p.incremental {
            set("incremental", i.to_string());
        }
        Ok(plan)
    }
}

pub fn is_nightly(rustc_vv: &str) -> bool {
    rustc_vv.lines().next().is_some_and(|l| l.contains("-nightly") || l.contains("-dev"))
}

/// Whether the toolchain ships the Cranelift backend (memoised per toolchain).
pub fn has_cranelift(input: PlanInput<'_>) -> bool {
    let rustc_vv = input.rustc_vv;
    static MEMO: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);
    if let Some(v) = memo.lock().unwrap_or_else(|p| p.into_inner()).get(rustc_vv) {
        return *v;
    }
    let host = rustc_vv.lines().find_map(|l| l.strip_prefix("host: ")).unwrap_or_default().trim();
    let sysroot = crate::builder::rustc_command(input.toolchain)
        .args(["--print", "sysroot"])
        .current_dir(input.dir)
        .stdin(Stdio::null())
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    let found = sysroot.is_some_and(|s| {
        std::fs::read_dir(s.join("lib/rustlib").join(host).join("codegen-backends"))
            .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().contains("rustc_codegen_cranelift")))
            .unwrap_or(false)
    });
    memo.lock().unwrap_or_else(|p| p.into_inner()).insert(rustc_vv.to_owned(), found);
    found
}

fn lld_name() -> &'static str {
    if cfg!(windows) {
        "lld-link"
    } else if cfg!(target_vendor = "apple") {
        "ld64.lld"
    } else {
        "ld.lld"
    }
}

/// Find an executable on `PATH`.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let p = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        p.is_file().then_some(p)
    })
}

fn toml_array(items: &[String]) -> String {
    // JSON string literals are valid TOML basic strings.
    let quoted: Vec<String> = items.iter().map(|s| serde_json::Value::String(s.clone()).to_string()).collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STABLE: &str = "rustc 1.97.1 (abc 2026-08-01)\nhost: x86_64-unknown-linux-gnu\n";
    const NIGHTLY: &str = "rustc 1.100.0-nightly (2e2b193f8 2026-09-02)\nhost: x86_64-unknown-linux-gnu\n";

    fn input(rustc_vv: &str) -> PlanInput<'_> {
        PlanInput {
            rustc_vv,
            toolchain: None,
            dir: Path::new("."),
            release: true,
            target: Target::Native,
            needs_unwind: false,
        }
    }

    #[test]
    fn nightly_only_features_are_refused_on_stable() {
        let a = Accel { threads: Some(8), sccache: Toggle::Off, ..Accel::default() };
        assert!(a.plan(input(STABLE)).unwrap_err().contains("nightly"));
        let p = a.plan(input(NIGHTLY)).unwrap();
        assert_eq!(p.rustflags, ["-Zthreads=8"]);
        let a = Accel { codegen: Codegen::Cranelift, sccache: Toggle::Off, ..Accel::default() };
        assert!(a.plan(input(STABLE)).unwrap_err().contains("nightly"));
    }

    #[test]
    fn profile_tuning_becomes_config_and_key_material() {
        let a = Accel {
            sccache: Toggle::Off,
            linker: LinkerChoice::Default,
            profile: ProfileTuning {
                opt_level: Some("s".into()),
                codegen_units: Some(16),
                lto: Some("thin".into()),
                ..ProfileTuning::default()
            },
            ..Accel::default()
        };
        a.validate().unwrap();
        let p = a.plan(input(STABLE)).unwrap();
        assert!(p.cargo_args.contains(&"profile.release.opt-level=\"s\"".to_owned()));
        assert!(p.cargo_args.contains(&"profile.release.codegen-units=16".to_owned()));
        assert!(p.key_material().contains("lto=\"thin\""));
        assert_eq!(p.wrapper, Wrapper::Disable, "the compiler cache is never key material");
        assert!(!p.key_material().contains("sccache"));
    }

    #[test]
    fn invalid_values_are_rejected() {
        let mut a = Accel::default();
        a.profile.lto = Some("yes".into());
        assert!(a.validate().is_err());
        let a = Accel { toolchain: Some("nightly; rm -rf /".into()), ..Accel::default() };
        assert!(a.validate().is_err());
        let a = Accel { threads: Some(0), ..Accel::default() };
        assert!(a.validate().is_err());
    }

    fn off(a: Accel) -> Accel {
        Accel { sccache: Toggle::Off, ..a }
    }

    /// A `rustc -vV` whose `host:` is an absolute directory: `has_cranelift`
    /// then looks for `<dir>/codegen-backends`, so the test decides whether
    /// the toolchain ships Cranelift. The temp dir keeps the memo key unique.
    fn nightly_with_cranelift(present: bool) -> (tempfile::TempDir, String) {
        let host = tempfile::tempdir().unwrap();
        let backends = host.path().join("codegen-backends");
        std::fs::create_dir(&backends).unwrap();
        if present {
            std::fs::write(backends.join("librustc_codegen_cranelift-nightly.so"), b"").unwrap();
        }
        let vv = format!("rustc 1.100.0-nightly (2e2b193f8 2026-09-02)\nhost: {}\n", host.path().display());
        (host, vv)
    }

    fn dev(rustc_vv: &str) -> PlanInput<'_> {
        PlanInput { release: false, ..input(rustc_vv) }
    }

    #[test]
    fn nightly_detection_reads_the_version_line() {
        assert!(is_nightly(NIGHTLY));
        assert!(is_nightly("rustc 1.101.0-dev\nhost: x\n"));
        assert!(!is_nightly(STABLE));
        assert!(!is_nightly("rustc 1.98.0-beta.3 (abc 2026-09-01)\n"));
        assert!(!is_nightly("rustc 1.97.1\nrelease: 1.97.1-nightly\n"), "only the first line counts");
        assert!(!is_nightly(""));
    }

    #[test]
    fn valid_values_pass_validation() {
        for tc in ["nightly", "1.97.1", "nightly-2026-09-30", "stable-aarch64-apple-darwin"] {
            Accel { toolchain: Some(tc.into()), ..Accel::default() }.validate().unwrap();
        }
        for o in ["0", "1", "2", "3", "s", "z"] {
            let mut a = Accel::default();
            a.profile.opt_level = Some(o.into());
            a.validate().unwrap();
        }
        let a = Accel {
            threads: Some(256),
            profile: ProfileTuning {
                lto: Some("fat".into()),
                strip: Some("symbols".into()),
                debug: Some(2),
                codegen_units: Some(1),
                incremental: Some(true),
                opt_level: None,
            },
            ..Accel::default()
        };
        a.validate().unwrap();
        Accel { threads: Some(1), ..Accel::default() }.validate().unwrap();
    }

    #[test]
    fn plan_revalidates_programmatic_settings() {
        let mut a = off(Accel::default());
        a.profile.strip = Some("everything".into());
        let err = a.plan(input(STABLE)).unwrap_err();
        assert!(err.contains("profile.strip `everything`"), "{err}");
    }

    #[test]
    fn default_plan_on_stable_changes_nothing() {
        let p = off(Accel { linker: LinkerChoice::Default, ..Accel::default() }).plan(input(STABLE)).unwrap();
        assert!(p.rustflags.is_empty() && p.cargo_args.is_empty() && p.applied.is_empty(), "{p:?}");
        assert_eq!(p.key_material(), "\n");
    }

    #[test]
    fn sccache_is_disabled_or_kept_but_never_key_material() {
        let p = off(Accel::default()).plan(input(STABLE)).unwrap();
        assert_eq!(p.wrapper, Wrapper::Disable);
        let base = Plan { rustflags: vec!["-Cx".into()], cargo_args: vec!["-Zy".into()], ..Plan::default() };
        for wrapper in [Wrapper::Keep, Wrapper::Disable, Wrapper::Set("/opt/bin/sccache".into())] {
            assert_eq!(Plan { wrapper, ..base.clone() }.key_material(), base.key_material());
        }
        if std::env::var_os("RUSTC_WRAPPER").is_none() && find_tool("sccache").is_none() {
            let err = Accel { sccache: Toggle::On, ..Accel::default() }.plan(input(STABLE)).unwrap_err();
            assert!(err.contains("sccache = \"on\" but sccache is not on PATH"), "{err}");
            let p = Accel::default().plan(input(STABLE)).unwrap();
            assert_eq!(p.wrapper, Wrapper::Keep, "auto skips a missing sccache");
        }
    }

    #[test]
    fn wrapper_is_applied_through_rustc_wrapper() {
        let env = |w: Wrapper| {
            let mut cmd = Command::new("cargo");
            Plan { wrapper: w, ..Plan::default() }.apply(&mut cmd);
            cmd.get_envs()
                .find(|(k, _)| *k == "RUSTC_WRAPPER")
                .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        assert_eq!(env(Wrapper::Keep), None);
        assert_eq!(env(Wrapper::Disable), Some(Some(String::new())));
        assert_eq!(env(Wrapper::Set("/bin/sccache".into())), Some(Some("/bin/sccache".into())));
    }

    #[test]
    fn explicit_linkers_fail_loudly_when_unavailable() {
        let plan = |linker| off(Accel { linker, ..Accel::default() }).plan(input(STABLE));
        if cfg!(target_os = "linux") {
            if find_tool("mold").is_none() {
                assert!(plan(LinkerChoice::Mold).unwrap_err().contains("mold is not on PATH"));
            }
        } else {
            assert_eq!(plan(LinkerChoice::Mold).unwrap_err(), "linker = \"mold\" is supported on Linux only");
        }
        match find_tool(lld_name()) {
            None => assert_eq!(
                plan(LinkerChoice::Lld).unwrap_err(),
                format!("linker = \"lld\" but {} is not on PATH", lld_name())
            ),
            Some(_) => {
                let p = plan(LinkerChoice::Lld).unwrap();
                let flag =
                    if cfg!(windows) { format!("-Clinker={}", lld_name()) } else { "-Clink-arg=-fuse-ld=lld".into() };
                assert_eq!(p.rustflags, [flag]);
                assert!(p.applied.contains(&"linker lld".to_owned()));
            }
        }
        assert!(plan(LinkerChoice::Default).unwrap().rustflags.is_empty());
        if !cfg!(target_os = "linux") {
            assert!(plan(LinkerChoice::Auto).unwrap().rustflags.is_empty(), "auto means mold on Linux only");
        }
    }

    #[test]
    fn linker_choice_does_not_apply_to_wasm() {
        for linker in [LinkerChoice::Mold, LinkerChoice::Lld, LinkerChoice::Auto] {
            let p = off(Accel { linker, ..Accel::default() })
                .plan(PlanInput { target: Target::Wasm, ..input(STABLE) })
                .unwrap();
            assert!(p.rustflags.is_empty(), "{linker:?}: {p:?}");
        }
    }

    #[test]
    fn explicit_cranelift_without_the_backend_is_an_error() {
        let (_host, vv) = nightly_with_cranelift(false);
        let err = off(Accel { codegen: Codegen::Cranelift, ..Accel::default() }).plan(dev(&vv)).unwrap_err();
        assert!(err.contains("toolchain lacks it") && err.contains("rustc-codegen-cranelift-preview"), "{err}");
        // `auto` silently stays on LLVM.
        let p = off(Accel { codegen: Codegen::Auto, ..Accel::default() }).plan(dev(&vv)).unwrap();
        assert!(p.cargo_args.is_empty() && p.applied.is_empty(), "{p:?}");
    }

    #[test]
    fn cranelift_is_used_only_for_native_artifacts_loaded_out_of_process() {
        let (_host, vv) = nightly_with_cranelift(true);
        for codegen in [Codegen::Cranelift, Codegen::Auto] {
            let a = off(Accel { codegen, ..Accel::default() });
            let p = a.plan(dev(&vv)).unwrap();
            assert_eq!(
                p.cargo_args,
                ["-Zcodegen-backend", "--config", "profile.dev.codegen-backend=\"cranelift\""],
                "{codegen:?}"
            );
            assert_eq!(p.applied, ["cranelift"]);

            // D22: in-process code must unwind, which Cranelift cannot do.
            let p = a.plan(PlanInput { needs_unwind: true, ..dev(&vv) }).unwrap();
            assert!(p.cargo_args.is_empty(), "{codegen:?}: {p:?}");
            assert_eq!(p.applied, ["llvm (cranelift does not unwind; in-process plugins need it)"]);

            let p = a.plan(PlanInput { target: Target::Wasm, ..dev(&vv) }).unwrap();
            assert!(p.cargo_args.is_empty(), "{codegen:?}: {p:?}");
            assert_eq!(p.applied, ["llvm (cranelift does not target wasm)"]);
        }
        // `auto` keeps release builds on LLVM; an explicit request applies to release too.
        let p = off(Accel { codegen: Codegen::Auto, ..Accel::default() }).plan(input(&vv)).unwrap();
        assert!(p.cargo_args.is_empty(), "{p:?}");
        let p = off(Accel { codegen: Codegen::Cranelift, ..Accel::default() }).plan(input(&vv)).unwrap();
        assert!(p.cargo_args.contains(&"profile.release.codegen-backend=\"cranelift\"".to_owned()), "{p:?}");
    }

    #[test]
    fn codegen_auto_on_stable_does_not_probe_or_fail() {
        let p = off(Accel { codegen: Codegen::Auto, ..Accel::default() }).plan(dev(STABLE)).unwrap();
        assert!(p.cargo_args.is_empty() && p.applied.is_empty(), "{p:?}");
    }

    #[test]
    fn profile_tuning_targets_the_active_profile_with_toml_values() {
        let a = off(Accel {
            linker: LinkerChoice::Default,
            profile: ProfileTuning {
                opt_level: Some("3".into()),
                codegen_units: Some(4),
                lto: Some("off".into()),
                debug: Some(1),
                strip: Some("debuginfo".into()),
                incremental: Some(false),
            },
            ..Accel::default()
        });
        let p = a.plan(dev(STABLE)).unwrap();
        let configs: Vec<&str> =
            p.cargo_args.chunks(2).map(|c| (c[0] == "--config").then_some(&*c[1]).unwrap()).collect();
        assert_eq!(
            configs,
            [
                "profile.dev.opt-level=3",
                "profile.dev.codegen-units=4",
                "profile.dev.lto=\"off\"",
                "profile.dev.debug=1",
                "profile.dev.strip=\"debuginfo\"",
                "profile.dev.incremental=false",
            ]
        );
        assert_eq!(p.applied.len(), 6);
        let release = a.plan(input(STABLE)).unwrap();
        assert!(release.cargo_args.contains(&"profile.release.opt-level=3".to_owned()));
        assert_ne!(p.key_material(), release.key_material(), "the profile is part of the key material");
    }

    #[test]
    fn key_material_changes_with_everything_that_changes_bytes() {
        let key = |a: Accel, i: PlanInput<'_>| off(a).plan(i).unwrap().key_material();
        let base = key(Accel::default(), input(NIGHTLY));
        let mut tuned = Accel::default();
        tuned.profile.lto = Some("fat".into());
        let variants = [
            key(Accel { threads: Some(2), ..Accel::default() }, input(NIGHTLY)),
            key(Accel { threads: Some(4), ..Accel::default() }, input(NIGHTLY)),
            key(tuned, input(NIGHTLY)),
        ];
        for v in &variants {
            assert_ne!(v, &base);
        }
        assert_ne!(variants[0], variants[1]);
        // Rustflags and cargo args are kept apart.
        let a = Plan { rustflags: vec!["a".into()], ..Plan::default() };
        let b = Plan { cargo_args: vec!["a".into()], ..Plan::default() };
        assert_ne!(a.key_material(), b.key_material());
    }

    #[test]
    fn toml_array_quotes_and_escapes() {
        assert_eq!(toml_array(&[]), "[]");
        assert_eq!(toml_array(&["-Cx".into(), "a\"b\\c".into()]), r#"["-Cx", "a\"b\\c"]"#);
    }

    #[test]
    fn rustflags_reach_cargo_via_config_when_env_is_empty() {
        let p = Plan { rustflags: vec!["-Zthreads=4".into()], ..Plan::default() };
        if std::env::var_os("RUSTFLAGS").is_none() && std::env::var_os("CARGO_ENCODED_RUSTFLAGS").is_none() {
            let mut cmd = Command::new("cargo");
            p.apply(&mut cmd);
            let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
            assert_eq!(args, ["--config", "build.rustflags=[\"-Zthreads=4\"]"]);
        }
    }
}
