//! `cargo plugwright` — build, inspect and call plugins from a `plugwright.toml`.
//!
//! ```text
//! cargo plugwright [--manifest PATH] [-v] build [ID...]       build or fetch from cache
//! cargo plugwright [--manifest PATH] [-v] list                manifest, lockfile and cache state
//! cargo plugwright [--manifest PATH] [-v] call ID METHOD [INPUT|-]
//! cargo plugwright [--manifest PATH] [-v] gc                  drop cache entries the lockfile does not use
//! cargo plugwright [--manifest PATH] [-v] doctor              toolchain, targets, accelerators, sandbox
//! cargo plugwright [-v] lib CRATE [--dynamic|--static] [--out-dir DIR]
//! cargo plugwright [-v] wasm CRATE [--out-dir DIR]
//! cargo plugwright [-v] plugin CRATE [--wasm] [--out-dir DIR]
//! ```
//!
//! Without `--manifest`, `plugwright.toml` is searched for upwards from the
//! current directory. `lib`, `wasm` and `plugin` need no manifest: they take a
//! crate of the cargo workspace around the current directory. `RUST_LOG`
//! overrides `-v`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use plugwright::accel::{self, PlanInput};
use plugwright::{
    BuildRequest, Builder, Cache, Isolation, ManagerOptions, Manifest, PluginError, PluginKind, PluginManager, Source,
    Target,
};
use tracing_subscriber::EnvFilter;

mod pack;

const USAGE: &str = "usage: cargo plugwright [--manifest PATH] [-v] <command>

commands:
  build [ID...]               build plugins (or fetch them from the cache)
  list                        plugins with their lockfile and cache state
  call ID METHOD [INPUT|-]    load one plugin and call it; `-` reads stdin
  methods ID                  list the methods a `#[plugin]` plugin exports
  gc                          remove cache entries the lockfile does not reference
  doctor                      check toolchain, targets, accelerators and sandbox

workspace crates (no manifest; output in --out-dir, default ./dist):
  lib CRATE [--dynamic|--static]  shared (default) or static library, whatever the crate declares
  wasm CRATE                      wasm32-unknown-unknown module
  plugin CRATE [--wasm]           the crate as a plugin, loaded once to check it";

fn main() -> ExitCode {
    // Isolated plugins re-exec this binary as their runner.
    plugwright::maybe_run_runner();

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `cargo plugwright …` runs `cargo-plugwright plugwright …`.
    if args.first().is_some_and(|a| a == "plugwright") {
        args.remove(0);
    }
    let mut manifest = None;
    let mut verbose = false;
    let mut rest = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--manifest" => match it.next() {
                Some(p) => manifest = Some(PathBuf::from(p)),
                None => return usage("--manifest needs a path"),
            },
            "-v" | "--verbose" => verbose = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("cargo-plugwright {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => rest.push(a),
        }
    }

    let default_filter = if verbose { "plugwright=info,warn" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)))
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let Some(cmd) = rest.first().cloned() else { return usage("no command given") };
    // Looked up only once the arguments are valid, so bad usage exits 2 even outside a project.
    let manifest = || manifest.clone().map_or_else(find_manifest, Ok);
    let res = match (cmd.as_str(), &rest[1..]) {
        ("build", ids) => manifest().and_then(|m| build(&m, ids)),
        ("list", []) => manifest().and_then(|m| list(&m)),
        ("call", [id, method]) => manifest().and_then(|m| call(&m, id, method, None)),
        ("call", [id, method, input]) => manifest().and_then(|m| call(&m, id, method, Some(input))),
        ("methods", [id]) => manifest().and_then(|m| methods(&m, id)),
        ("gc", []) => manifest().and_then(|m| gc(&m)),
        ("doctor", []) => manifest().and_then(|m| doctor(&m)),
        ("lib" | "wasm" | "plugin", args) => match pack::parse(&cmd, args) {
            Ok(a) => pack::pack(&a),
            Err(why) => return usage(&why),
        },
        ("list" | "call" | "methods" | "gc" | "doctor", _) => return usage(&format!("bad arguments for `{cmd}`")),
        _ => return usage(&format!("unknown command `{cmd}`")),
    };
    match res {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => fail(e),
    }
}

fn usage(why: &str) -> ExitCode {
    eprintln!("error: {why}\n\n{USAGE}");
    ExitCode::from(2)
}

fn fail(e: PluginError) -> ExitCode {
    eprintln!("error: {e}");
    ExitCode::FAILURE
}

/// `plugwright.toml` in the current directory or the nearest parent.
fn find_manifest() -> Result<PathBuf, PluginError> {
    let cwd = std::env::current_dir().map_err(|e| PluginError::Manifest(format!("current directory: {e}")))?;
    cwd.ancestors()
        .map(|d| d.join("plugwright.toml"))
        .find(|p| p.is_file())
        .ok_or_else(|| PluginError::Manifest(format!("no plugwright.toml in {} or its parents", cwd.display())))
}

fn build(manifest: &Path, ids: &[String]) -> Result<bool, PluginError> {
    let (mut mgr, m) = PluginManager::from_manifest(manifest)?;
    if let Some(unknown) = ids.iter().find(|id| !m.plugins.iter().any(|p| &p.id == *id)) {
        return Err(PluginError::NotFound(unknown.clone()));
    }
    let mut ok = true;
    for s in m.plugins.iter().filter(|s| if ids.is_empty() { s.enabled } else { ids.contains(&s.id) }) {
        match mgr.prepare(s) {
            Ok(a) => println!("  ✓ {:<20} {} ({})", s.id, a.path.display(), if a.cached { "cached" } else { "built" }),
            Err(e) => {
                ok = false;
                println!("  ✗ {:<20} {e}", s.id);
            }
        }
    }
    Ok(ok)
}

fn list(manifest: &Path) -> Result<bool, PluginError> {
    let (mgr, m) = PluginManager::from_manifest(manifest)?;
    println!("{:<20} {:<7} {:<11} {:<8} {:<14} source", "id", "kind", "run", "cache", "sha256");
    for s in &m.plugins {
        let run = match s.kind {
            PluginKind::Native => match s.effective_isolation() {
                Isolation::InProcess => "in-process".to_owned(),
                Isolation::Process => "process".to_owned(),
            },
            PluginKind::Wasm => s.engine.clone().unwrap_or_else(|| "wasm".into()),
        };
        let lock = mgr.lockfile().plugins.iter().find(|l| l.id == s.id);
        let cache = match lock.and_then(|l| l.cache_key.as_deref()) {
            Some(key) => match mgr.cache().lookup(key) {
                Ok(Some(_)) => "cached",
                _ => "missing",
            },
            None if matches!(s.source, Source::Artifact(_)) => "prebuilt",
            None => "-",
        };
        let sha = lock.map_or("-", |l| &l.sha256[..12.min(l.sha256.len())]);
        let source = match (&s.source, lock) {
            (_, Some(l)) => l.source.clone(),
            (Source::Path(p), None) => format!("path+{}", p.display()),
            (Source::Artifact(p), None) => format!("artifact+{}", p.display()),
            (Source::Git(g), None) => g.label("").trim_end_matches('#').to_owned(),
        };
        let id = if s.enabled { s.id.clone() } else { format!("{} (off)", s.id) };
        println!("{id:<20} {:<7} {run:<11} {cache:<8} {sha:<14} {source}", format!("{:?}", s.kind).to_lowercase());
    }
    Ok(true)
}

fn call(manifest: &Path, id: &str, method: &str, input: Option<&String>) -> Result<bool, PluginError> {
    let input = match input.map(String::as_str) {
        Some("-") => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).map_err(|e| PluginError::Manifest(format!("stdin: {e}")))?;
            buf
        }
        Some(s) => s.as_bytes().to_vec(),
        None => Vec::new(),
    };
    let mut mgr = load_one(manifest, id)?;
    let out = mgr.call(id, method, &input)?;
    let mut stdout = std::io::stdout().lock();
    // Binary-safe; a trailing newline only for text that lacks one.
    let _ = stdout.write_all(&out);
    if std::str::from_utf8(&out).is_ok_and(|s| !s.ends_with('\n')) {
        let _ = stdout.write_all(b"\n");
    }
    Ok(true)
}

fn load_one(manifest: &Path, id: &str) -> Result<PluginManager, PluginError> {
    let (mut mgr, m) = PluginManager::from_manifest(manifest)?;
    let spec = m.plugins.into_iter().find(|s| s.id == id).ok_or_else(|| PluginError::NotFound(id.into()))?;
    mgr.load(spec)?;
    Ok(mgr)
}

fn methods(manifest: &Path, id: &str) -> Result<bool, PluginError> {
    for m in load_one(manifest, id)?.describe(id)? {
        println!("{}({})", m.name, m.params.join(", "));
    }
    Ok(true)
}

fn gc(manifest: &Path) -> Result<bool, PluginError> {
    let (mgr, _) = PluginManager::from_manifest(manifest)?;
    println!("removed {} cache entries", mgr.gc_cache()?);
    Ok(true)
}

/// One line of the doctor report.
fn row(ok: bool, what: &str, detail: impl std::fmt::Display) {
    println!("  {} {what:<12} {detail}", if ok { "✓" } else { "!" });
}

fn doctor(manifest: &Path) -> Result<bool, PluginError> {
    let m = Manifest::load(manifest)?;
    let opts = ManagerOptions::from_manifest(&m);
    let dir = m.base_dir.clone();
    let toolchain = opts.build.accel.toolchain.as_deref();
    let tc_arg = toolchain.map(|t| format!("+{t}"));
    let run = |prog: &str, args: &[&str]| -> Option<String> {
        let mut c = Command::new(prog);
        c.args(tc_arg.as_deref()).args(args).current_dir(&dir);
        let out = c.output().ok().filter(|o| o.status.success())?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let mut healthy = true;
    println!("plugwright doctor ({})", manifest.display());

    let Some(vv) = run("rustc", &["-vV"]) else {
        row(false, "rustc", format!("cannot run rustc {}", tc_arg.as_deref().unwrap_or("")));
        return Ok(false);
    };
    let host = vv.lines().find_map(|l| l.strip_prefix("host: ")).unwrap_or("?").to_owned();
    row(true, "rustc", format!("{} ({host})", vv.lines().next().unwrap_or("?")));
    row(run("cargo", &["-V"]).is_some(), "cargo", run("cargo", &["-V"]).unwrap_or_else(|| "not found".into()));

    let sysroot = run("rustc", &["--print", "sysroot"]).map(PathBuf::from);
    let has_target = |t: &str| sysroot.as_ref().is_some_and(|s| s.join("lib/rustlib").join(t).join("lib").is_dir());
    let wasm = has_target("wasm32-unknown-unknown");
    let needs_wasm = m.plugins.iter().any(|p| p.kind == PluginKind::Wasm && !matches!(p.source, Source::Artifact(_)));
    healthy &= wasm || !needs_wasm;
    row(
        wasm,
        "wasm target",
        if wasm {
            "wasm32-unknown-unknown".into()
        } else {
            "missing: rustup target add wasm32-unknown-unknown".to_owned()
        },
    );

    match plugwright::relink::host_linker(&dir) {
        Ok(l) => row(true, "relink", format!("static libraries link with {l}")),
        Err(e) => row(false, "relink", format!("static libraries cannot be relinked: {e}")),
    }

    let engines = plugwright::builtin_engines();
    let names: Vec<&str> = engines.iter().map(|e| e.name()).collect();
    row(!names.is_empty(), "wasm engines", names.join(", "));

    let sandbox = plugwright::sandbox::capabilities();
    row(sandbox.gaps.is_empty(), "sandbox", &sandbox);

    let tool = |name: &str| accel::find_tool(name).map_or_else(|| "not found".into(), |p| p.display().to_string());
    row(true, "sccache", tool("sccache"));
    if cfg!(target_os = "linux") {
        row(true, "mold", tool("mold"));
    }
    let nightly = accel::is_nightly(&vv);
    let input = PlanInput {
        rustc_vv: &vv,
        toolchain,
        dir: &dir,
        release: opts.build.release,
        target: Target::Native,
        needs_unwind: false,
    };
    let clif = nightly && accel::has_cranelift(input);
    row(
        true,
        "cranelift",
        match (nightly, clif) {
            (false, _) => "needs a nightly toolchain ([settings.build] toolchain = \"nightly\")".to_owned(),
            (true, false) => "rustup component add rustc-codegen-cranelift-preview --toolchain nightly".to_owned(),
            (true, true) => "available (isolated plugins only: it does not unwind)".to_owned(),
        },
    );

    // What `[settings.build]` resolves to for a native plugin built here.
    let builder = Builder::new(Cache::new(&opts.cache_dir)?, opts.build.clone());
    for (label, in_process) in [("accel", false), ("in-process", true)] {
        let req = BuildRequest {
            id: "doctor".into(),
            crate_dir: dir.clone(),
            target: Target::Native,
            features: vec![],
            no_default_features: false,
            source_label: String::new(),
            in_process,
            crate_type: None,
        };
        match builder.plan(&req) {
            Ok(p) if p.applied.is_empty() => row(true, label, "none"),
            Ok(p) => row(true, label, p.applied.join(", ")),
            Err(e) => {
                healthy = false;
                row(false, label, e);
            }
        }
    }
    println!("{}", if healthy { "ok" } else { "problems found" });
    Ok(healthy)
}
