//! End-to-end tests: real cargo builds, real dlopen, real child processes,
//! real Wasm engines. The cache lives in `target/tmp`, so only the first run
//! pays for compilation.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use plugwright::builder::{BuildOptions, BuildRequest, Builder, Target};
use plugwright::{
    Cache, Isolation, ManagerOptions, PluginError, PluginKind, PluginManager, PluginSpec, Release, RunnerCommand,
    SandboxPolicy, Source, WasmError, WatchOptions,
};

/// `assert!(matches!(r, Err(p)))`, but a failure shows what came back instead.
macro_rules! assert_err {
    ($e:expr, $p:pat) => {{
        let r = $e;
        assert!(matches!(r, Err($p)), "expected Err({}), got {:?}", stringify!($p), r);
    }};
}

fn workspace() -> PathBuf {
    fs_kit::path::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

fn manager() -> PluginManager {
    PluginManager::new(options()).unwrap()
}

fn options() -> ManagerOptions {
    let mut o = ManagerOptions::new(workspace().join("examples"));
    o.cache_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e");
    o.isolation.runner = RunnerCommand::Path(env!("CARGO_BIN_EXE_plugwright-host").into());
    o.isolation.call_timeout = Duration::from_millis(1500);
    o.isolation.max_restarts = 2;
    o
}

fn native(id: &str, isolation: Isolation) -> PluginSpec {
    let mut s = PluginSpec::new(id, PluginKind::Native, Source::Path("native-hello".into()));
    s.isolation = Some(isolation);
    s
}

fn wasm(id: &str, engine: &str) -> PluginSpec {
    let mut s = PluginSpec::new(id, PluginKind::Wasm, Source::Path("wasm-hello".into()));
    s.engine = Some(engine.into());
    s.limits.fuel = Some(5_000_000);
    s.limits.max_memory_bytes = Some(8 << 20);
    s.limits.timeout = Some(Duration::from_millis(500));
    s
}

fn text(r: Result<Vec<u8>, PluginError>) -> String {
    String::from_utf8(r.unwrap()).unwrap()
}

#[test]
fn native_in_process_survives_panics_and_errors() {
    let mut m = manager();
    m.load(native("n", Isolation::InProcess)).unwrap();
    assert!(text(m.call("n", "greet", b"x")).starts_with("Hello, x!"));
    let sum: serde_json::Value = m.call_json("n", "add", &serde_json::json!({"a": 1, "b": 2})).unwrap();
    assert_eq!(sum["sum"], 3);
    assert_err!(m.call("n", "panic", b""), PluginError::Panicked(_));
    assert_err!(m.call("n", "nope", b""), PluginError::Call { .. });
    assert!(text(m.call("n", "greet", b"y")).contains("call #2"), "state survives a caught panic");
    m.unload("n").unwrap();
    assert_err!(m.call("n", "greet", b""), PluginError::NotFound(_));
}

/// `unload` reports what it could verify, and the manager counts leaks.
#[test]
fn unload_reports_whether_code_left_the_process() {
    let mut m = manager();
    m.load(native("u-in", Isolation::InProcess)).unwrap();
    m.load(native("u-iso", Isolation::Process)).unwrap();
    m.load(wasm("u-wasm", "wasmi")).unwrap();
    assert_eq!(m.list().count(), 3);

    let native = m.unload("u-in").unwrap();
    eprintln!("in-process unload: {native}");
    assert!(matches!(native, Release::Unmapped | Release::StillMapped));
    assert_eq!(m.leaked_images(), usize::from(native == Release::StillMapped));
    assert_eq!(m.unload("u-iso").unwrap(), Release::ProcessExited);
    assert_eq!(m.unload("u-wasm").unwrap(), Release::Freed);
    assert_err!(m.unload("u-in"), PluginError::NotFound(_));
}

#[test]
fn isolated_plugin_survives_crashes_and_timeouts() {
    let mut m = manager();
    m.load(native("iso", Isolation::Process)).unwrap();
    let first = text(m.call("iso", "greet", b"a"));
    assert!(!first.contains(&format!("pid {})", std::process::id())), "runs in another process");
    assert_eq!(text(m.call("iso", "print", b"")), "printed", "stray stdout must not break the protocol");

    assert_err!(m.call("iso", "crash", b""), PluginError::Crashed(_));
    assert!(text(m.call("iso", "greet", b"b")).contains("call #1"), "restarted with fresh state");

    assert_err!(m.call("iso", "sleep", b"10000"), PluginError::Timeout(_));
    assert!(text(m.call("iso", "greet", b"c")).starts_with("Hello, c!"));

    // Budget of 2 restarts per window is now spent.
    assert_err!(m.call("iso", "abort", b""), PluginError::Crashed(_));
    assert_err!(m.call("iso", "greet", b""), PluginError::Disabled(_));
}

/// The runner maps the region before LOAD confines it, and keeps using it after.
#[test]
fn large_payloads_go_through_shared_memory_under_the_sandbox() {
    use plugwright::{IsolatedPlugin, NativeLoader, Plugin};
    let o = options();
    let artifact =
        NativeLoader::new(&o.cache_dir).unwrap().build(workspace().join("examples/native-hello")).unwrap().path;
    let policy = SandboxPolicy::default();
    let mut p =
        IsolatedPlugin::spawn("shm", &artifact, b"{}", o.isolation.clone(), policy, Arc::new(|_, _| {})).unwrap();
    assert!(p.info().sandbox.is_some(), "sandboxed");
    assert_eq!(p.shared_memory(), Some(o.isolation.shm_size));

    for len in [o.isolation.shm_threshold - 1, o.isolation.shm_threshold, 4 << 20] {
        let name = "x".repeat(len);
        let out = String::from_utf8(p.call("greet", name.as_bytes()).unwrap()).unwrap();
        assert!(out.starts_with(&format!("Hello, {name}!")), "{len} bytes");
    }
    // Larger than the region: the pipe carries it.
    let name = "y".repeat(o.isolation.shm_size + 1);
    assert!(String::from_utf8(p.call("greet", name.as_bytes()).unwrap()).unwrap().contains(&name));
    assert_eq!(p.shared_memory(), Some(o.isolation.shm_size), "the runner survived every call");
}

/// Spawn a TCP listener so `connect` has a reachable target.
fn listener() -> (std::net::TcpListener, String) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    (l, addr)
}

#[test]
fn sandbox_denies_writes_network_exec_and_foreign_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let secret = tmp.path().join("secret.txt");
    std::fs::write(&secret, b"top secret").unwrap();
    let out = tmp.path().join("out.txt");
    let (_l, addr) = listener();
    let probe = |m: &mut PluginManager, id: &str, method: &str, input: &str| m.call(id, method, input.as_bytes());

    let mut m = manager();
    m.load(native("boxed", Isolation::Process)).unwrap();
    let report = m.list().find(|(id, ..)| *id == "boxed").unwrap().1.sandbox.clone();
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        let report = report.expect("sandbox report");
        assert!(!report.contains("not enforced: file"), "{report}");
        for (method, input) in [
            ("read_file", secret.to_str().unwrap()),
            ("write_file", out.to_str().unwrap()),
            ("connect", addr.as_str()),
            ("spawn", ""),
        ] {
            let r = probe(&mut m, "boxed", method, input);
            assert!(matches!(r, Err(PluginError::Call { .. })), "{method} must be denied, got {r:?}");
        }
        assert!(!out.exists());
        // The plugin keeps working after denied operations.
        assert!(text(m.call("boxed", "greet", b"x")).starts_with("Hello, x!"));
    }

    // Opt-ins: read/write paths and network.
    let mut s = native("opened", Isolation::Process);
    s.sandbox.read = vec![tmp.path().to_owned()];
    s.sandbox.write = vec![tmp.path().to_owned()];
    s.sandbox.network = true;
    m.load(s).unwrap();
    assert_eq!(text(probe(&mut m, "opened", "read_file", secret.to_str().unwrap())), "read 10 bytes");
    // Windows' low integrity level cannot open single medium-integrity paths; the sandbox
    // reports that gap, and then the write must stay denied rather than silently succeed.
    let opened = m.list().find(|(id, ..)| *id == "opened").unwrap().1.sandbox.clone().unwrap_or_default();
    let write = probe(&mut m, "opened", "write_file", out.to_str().unwrap());
    if opened.contains("not enforced:") && opened.contains("write allow-list") {
        assert!(matches!(write, Err(PluginError::Call { .. })), "{opened}: {write:?}");
        assert!(!out.exists());
    } else {
        assert_eq!(text(write), "wrote", "{opened}");
    }
    assert_eq!(text(probe(&mut m, "opened", "connect", &addr)), "connected");

    // Disabled sandbox: processes can be spawned again.
    let mut s = native("free", Isolation::Process);
    s.sandbox = SandboxPolicy::disabled();
    m.load(s).unwrap();
    assert_eq!(m.list().find(|(id, ..)| *id == "free").unwrap().1.sandbox, None);
    #[cfg(unix)]
    assert_eq!(text(probe(&mut m, "free", "spawn", "")), "spawned");
}

fn exercise_wasm(engine: &str) {
    let mut m = manager();
    let id = format!("w-{engine}");
    m.load(wasm(&id, engine)).unwrap();
    assert_eq!(m.list().next().unwrap().1.engine.as_deref(), Some(engine));
    assert!(text(m.call(&id, "greet", b"w")).starts_with("Hello, w!"));
    assert_eq!(text(m.call(&id, "reverse", b"abc")), "cba");

    let looped = m.call(&id, "loop", b"").unwrap_err();
    assert!(matches!(looped, PluginError::Wasm(WasmError::OutOfFuel | WasmError::Timeout)), "{engine}: {looped}");
    let oom = m.call(&id, "oom", b"").unwrap_err();
    assert!(matches!(oom, PluginError::Wasm(WasmError::MemoryLimit)), "{engine}: {oom}");
    let trap = m.call(&id, "trap", b"").unwrap_err();
    assert!(matches!(trap, PluginError::Wasm(WasmError::Trap(_))), "{engine}: {trap}");
    assert!(text(m.call(&id, "greet", b"again")).contains("call #1"), "fresh instance after trap");
}

#[test]
fn wasm_on_wasmtime() {
    exercise_wasm("wasmtime");
}

#[test]
fn wasm_on_wasmi() {
    exercise_wasm("wasmi");
}

/// Each plugin gets the host functions its factory builds for its id.
#[test]
fn wasm_guests_call_host_functions_on_every_engine() {
    use plugwright::HostFunctions;
    let mut m = manager();
    m.set_host_functions(|id| {
        let id = id.to_owned();
        HostFunctions::new()
            .with("upper", |i: &[u8]| Ok(i.to_ascii_uppercase()))
            .with("whoami", move |_: &[u8]| Ok(id.clone().into_bytes()))
            .with("refuse", |i: &[u8]| Err(format!("refused {}", String::from_utf8_lossy(i))))
    });
    for engine in ["wasmtime", "wasmi"] {
        let id = format!("h-{engine}");
        m.load(wasm(&id, engine)).unwrap_or_else(|e| panic!("{engine}: {e}"));
        let ask = |m: &mut PluginManager, name: &str, input: &str| {
            m.call(&id, "ask", format!(r#"["{name}","{input}"]"#).as_bytes())
        };
        assert_eq!(text(ask(&mut m, "upper", "abc")), "ABC", "{engine}");
        assert_eq!(text(ask(&mut m, "whoami", "")), id, "{engine}");
        for (name, want) in [("refuse", "refused x"), ("nope", "unknown host function `nope`")] {
            match ask(&mut m, name, "x") {
                Err(PluginError::Call { message, .. }) => assert_eq!(message, want, "{engine}"),
                other => panic!("{engine}: {other:?}"),
            }
        }
    }
}

/// With practically unlimited fuel, only the wall clock can stop `loop`.
#[test]
fn wasm_wall_clock_timeout_on_every_engine() {
    let mut m = manager();
    for engine in ["wasmtime", "wasmi"] {
        let id = format!("t-{engine}");
        let mut s = wasm(&id, engine);
        s.limits.fuel = Some(u64::MAX / 2);
        s.limits.timeout = Some(Duration::from_millis(300));
        m.load(s).unwrap_or_else(|e| panic!("{engine}: {e}"));
        let started = std::time::Instant::now();
        let r = m.call(&id, "loop", b"");
        let took = started.elapsed();
        assert!(matches!(r, Err(PluginError::Wasm(WasmError::Timeout))), "{engine}: {r:?}");
        assert!(took < Duration::from_millis(800), "{engine}: stopped after {took:?}");
        assert!(text(m.call(&id, "greet", b"t")).starts_with("Hello, t!"), "{engine}: usable after timeout");
    }
}

#[test]
fn version_mismatch_is_an_error() {
    let mut m = manager();
    let mut s = native("v", Isolation::InProcess);
    s.version = Some("^9".parse().unwrap());
    assert_err!(m.load(s), PluginError::VersionMismatch { .. });
    assert_eq!(m.list().count(), 0);
}

#[test]
fn unknown_engine_and_duplicate_ids_are_errors() {
    let mut m = manager();
    assert_err!(m.load(wasm("e", "nonexistent")), PluginError::Unsupported(_));
    m.load(native("d", Isolation::InProcess)).unwrap();
    assert_err!(m.load(native("d", Isolation::InProcess)), PluginError::AlreadyLoaded(_));
}

#[test]
fn prebuilt_artifact_is_integrity_checked() {
    let mut m = manager();
    let built = m.prepare(&native("p", Isolation::InProcess)).unwrap();
    let mut s = PluginSpec::new("art", PluginKind::Native, Source::Artifact(built.path.clone()));
    s.sha256 = Some("0".repeat(64));
    assert_err!(m.load(s.clone()), PluginError::Integrity { .. });
    s.sha256 = Some(built.sha256.clone());
    m.load(s).unwrap();
    assert!(text(m.call("art", "greet", b"z")).starts_with("Hello, z!"));
}

/// Scratch plugin crate outside the workspace.
fn scratch_crate(dir: &Path, lib_rs: &str, dep: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let root = workspace();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"scratch\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n\
             [dependencies]\n{dep} = {{ path = {} }}\n\n[workspace]\n",
            // A TOML string, escaped: Windows paths are full of `\\`.
            toml::Value::String(root.join("crates").join(dep).to_string_lossy().into_owned())
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), lib_rs).unwrap();
}

#[test]
fn cache_hits_and_invalidates_on_source_change() {
    let tmp = tempfile::tempdir().unwrap();
    let krate = tmp.path().join("scratch");
    let body = |n: u32| {
        format!(
            "struct P;\nimpl plugwright_sdk::Plugin for P {{\n  fn new(_: &[u8]) -> Result<Self, String> {{ Ok(P) }}\n  \
             fn call(&mut self, _: &str, _: &[u8]) -> Result<Vec<u8>, String> {{ Ok(b\"{n}\".to_vec()) }}\n}}\n\
             plugwright_sdk::export_plugin!(P);\n"
        )
    };
    scratch_crate(&krate, &body(1), "plugwright-sdk");
    let cache = Cache::new(Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e-scratch")).unwrap();
    let builder = Builder::new(cache, BuildOptions { release: false, ..BuildOptions::default() });
    let req = BuildRequest {
        id: "scratch".into(),
        crate_dir: krate.clone(),
        target: Target::Native,
        features: vec![],
        no_default_features: false,
        source_label: "test".into(),
        in_process: true,
        crate_type: None,
    };
    let a = builder.build(&req).unwrap();
    let b = builder.build(&req).unwrap();
    assert!(b.cached, "second build must be a cache hit");
    assert_eq!(a.sha256, b.sha256);

    std::fs::write(krate.join("src/lib.rs"), body(2)).unwrap();
    let c = builder.build(&req).unwrap();
    assert!(!c.cached, "a source edit must invalidate the key");
    assert_ne!(c.key, a.key);
}

/// A reload whose new build fails to load keeps the old instance serving and
/// leaves its lockfile pin (and so its cache entry) alone.
#[test]
fn failed_reload_keeps_the_previous_pin() {
    // A fixed crate path and cache keep the keys stable, so reruns hit the cache.
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e-pin");
    let krate = base.join("scratch");
    let body = |ctor: &str| {
        format!(
            "struct P;\nimpl plugwright_sdk::Plugin for P {{\n  fn new(_: &[u8]) -> Result<Self, String> {{ {ctor} }}\n  \
             fn call(&mut self, _: &str, _: &[u8]) -> Result<Vec<u8>, String> {{ Ok(b\"v1\".to_vec()) }}\n}}\n\
             plugwright_sdk::export_plugin!(P);\n"
        )
    };
    scratch_crate(&krate, &body("Ok(P)"), "plugwright-sdk");
    let mut o = options();
    o.cache_dir = base.join("cache");
    o.build.release = false;
    let mut m = PluginManager::new(o).unwrap();
    let mut spec = PluginSpec::new("pin", PluginKind::Native, Source::Path(krate.clone()));
    spec.isolation = Some(Isolation::InProcess);
    m.load(spec).unwrap();
    let good = m.lockfile().get("pin").cloned().unwrap();

    std::fs::write(krate.join("src/lib.rs"), body("Err(\"broken\".into())")).unwrap();
    let err = m.reload("pin").unwrap_err();
    assert!(err.to_string().contains("broken"), "{err}");
    assert_eq!(m.lockfile().get("pin"), Some(&good), "the failed build must not replace the pin");
    assert_eq!(text(m.call("pin", "x", b"")), "v1", "the old instance keeps serving");
    m.gc_cache().unwrap();
    assert!(m.cache().lookup(good.cache_key.as_deref().unwrap()).unwrap().is_some(), "gc keeps the pinned build");
}

#[test]
fn abi_mismatch_is_rejected_before_use() {
    let tmp = tempfile::tempdir().unwrap();
    let krate = tmp.path().join("bad-abi");
    scratch_crate(
        &krate,
        r#"
use plugwright_abi::{PwPluginDecl, PwStr};
static DECL: PwPluginDecl = PwPluginDecl {
    abi_version: 999, struct_size: 0, sdk_version: PwStr::EMPTY, name: PwStr::EMPTY, version: PwStr::EMPTY,
    create: None, call: None, destroy: None, free_buf: None,
};
#[unsafe(no_mangle)]
pub extern "C" fn plugwright_entry_v1() -> *const PwPluginDecl { &DECL }
"#,
        "plugwright-abi",
    );
    let mut m = manager();
    let mut s = PluginSpec::new("bad", PluginKind::Native, Source::Path(krate));
    s.isolation = Some(Isolation::InProcess);
    match m.load(s) {
        Err(PluginError::AbiMismatch { expected: 1, found: 999 }) => {}
        other => panic!("expected AbiMismatch, got {:?}", other.map(|i| i.name.clone())),
    }
}

/// A `crate-type = ["staticlib"]` plugin is built, relinked into a shared
/// library (cached), and runs in-process and isolated.
#[test]
fn static_library_is_relinked_and_loaded() {
    let mut m = manager();
    for (id, isolation) in [("s-in", Isolation::InProcess), ("s-proc", Isolation::Process)] {
        let mut s = PluginSpec::new(id, PluginKind::Native, Source::Path("static-hello".into()));
        s.isolation = Some(isolation);
        m.load(s).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(text(m.call(id, "greet", b"x")), "Hello from a static library, x!");
    }
    let spec = PluginSpec::new("s", PluginKind::Native, Source::Path("static-hello".into()));
    let a = m.prepare(&spec).unwrap();
    let b = m.prepare(&spec).unwrap();
    assert!(b.cached, "the relinked library must come from the cache");
    assert_eq!(a.sha256, b.sha256);
}

/// Prebuilt artifacts are recognised by content, not by file name: a bare
/// `.a`, a versioned `libx.so.1`-style name, and wrong kinds.
#[test]
fn prebuilt_formats_are_detected_by_content() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = Cache::new(Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e")).unwrap();
    let builder = Builder::new(cache, BuildOptions::default());
    let req = |id: &str, dir: &str, target| BuildRequest {
        id: id.into(),
        crate_dir: workspace().join("examples").join(dir),
        target,
        features: vec![],
        no_default_features: false,
        source_label: "test".into(),
        in_process: true,
        crate_type: None,
    };
    let archive = builder.build(&req("static", "static-hello", Target::Native)).unwrap();
    let shared = builder.build(&req("shared", "native-hello", Target::Native)).unwrap();
    let module = builder.build(&req("wasm", "wasm-hello", Target::Wasm)).unwrap();

    let copy = |from: &Path, name: &str| {
        let to = tmp.path().join(name);
        std::fs::copy(from, &to).unwrap();
        to
    };
    let artifact = |id: &str, kind, path: PathBuf| {
        let mut s = PluginSpec::new(id, kind, Source::Artifact(path));
        if kind == PluginKind::Native {
            s.isolation = Some(Isolation::InProcess);
        }
        s
    };
    let mut m = manager();
    m.load(artifact("a", PluginKind::Native, copy(&archive.path, "libstatic.a"))).unwrap();
    assert!(text(m.call("a", "greet", b"y")).contains("static library"));
    m.load(artifact("v", PluginKind::Native, copy(&shared.path, "libhello.so.1"))).unwrap();
    assert!(text(m.call("v", "greet", b"z")).starts_with("Hello, z!"));

    let not_a_plugin = |r: Result<&plugwright::PluginInfo, PluginError>| match r {
        Err(PluginError::NotAPlugin { reason, .. }) => reason,
        other => panic!("expected NotAPlugin, got {:?}", other.map(|i| i.name.clone())),
    };
    let reason = not_a_plugin(m.load(artifact("w", PluginKind::Native, copy(&module.path, "plugin.so"))));
    assert!(reason.contains("wasm"), "{reason}");
    let reason = not_a_plugin(m.load(artifact("n", PluginKind::Wasm, copy(&shared.path, "plugin.wasm"))));
    assert!(reason.contains("kind = \"wasm\""), "{reason}");
    std::fs::write(tmp.path().join("notes.dll"), b"just text").unwrap();
    not_a_plugin(m.load(artifact("t", PluginKind::Native, tmp.path().join("notes.dll"))));
}

/// Cranelift builds isolated plugins only: its code aborts on panic instead
/// of unwinding, which in-process would kill the host. Needs a nightly
/// toolchain with `rustc-codegen-cranelift-preview`; skipped otherwise.
#[test]
fn cranelift_is_used_for_isolated_plugins_only() {
    let sysroot = std::process::Command::new("rustc").args(["+nightly", "--print", "sysroot"]).output();
    let has = sysroot.ok().filter(|o| o.status.success()).is_some_and(|o| {
        let root = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        std::fs::read_dir(root.join("lib/rustlib"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|host| host.path().join("codegen-backends").is_dir())
    });
    if !has {
        eprintln!("skipping: no nightly toolchain with the Cranelift backend");
        return;
    }
    let mut o = options();
    o.cache_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e-clif");
    o.build.release = false;
    o.build.accel.toolchain = Some("nightly".into());
    o.build.accel.codegen = plugwright::accel::Codegen::Cranelift;

    let builder = Builder::new(Cache::new(&o.cache_dir).unwrap(), o.build.clone());
    let req = |in_process| BuildRequest {
        id: "c".into(),
        crate_dir: workspace().join("examples/native-hello"),
        target: Target::Native,
        features: vec![],
        no_default_features: false,
        source_label: "test".into(),
        in_process,
        crate_type: None,
    };
    assert!(builder.plan(&req(false)).unwrap().applied.iter().any(|a| a == "cranelift"));
    assert!(!builder.plan(&req(true)).unwrap().applied.iter().any(|a| a == "cranelift"));
    assert_ne!(builder.cache_key(&req(false)).unwrap(), builder.cache_key(&req(true)).unwrap());

    let mut m = PluginManager::new(o).unwrap();
    m.load(native("clif", Isolation::Process)).unwrap();
    assert!(text(m.call("clif", "greet", b"a")).starts_with("Hello, a!"));
    // A Cranelift panic aborts the runner; the host sees a crash and restarts it.
    assert_err!(m.call("clif", "panic", b""), PluginError::Crashed(_));
    assert!(text(m.call("clif", "greet", b"b")).contains("call #1"));

    m.load(native("llvm", Isolation::InProcess)).unwrap();
    assert_err!(m.call("llvm", "panic", b""), PluginError::Panicked(_));
}

/// The host side of the `#[plugin]` wire convention: a typed client.
#[plugwright::interface]
trait Greeter {
    fn greet(&mut self, name: &str) -> plugwright::Result<String>;
    fn add(&mut self, a: i64, b: i64) -> plugwright::Result<Sum>;
    #[plugwright(name = "panic")]
    fn boom(&mut self) -> plugwright::Result<()>;
    fn missing(&mut self, x: Vec<u8>) -> plugwright::Result<Vec<u8>>;

    /// Provided methods run on the host and are not forwarded.
    fn shout(&mut self, name: &str) -> plugwright::Result<String> {
        Ok(self.greet(name)?.to_uppercase())
    }
}

#[derive(serde::Deserialize, Debug, PartialEq)]
struct Sum {
    sum: i64,
}

#[test]
fn typed_interface_over_native_and_wasm() {
    let mut m = manager();
    m.load(native("typed", Isolation::InProcess)).unwrap();
    {
        let mut g = GreeterClient::new(m.bind("typed").unwrap());
        assert!(g.greet("x").unwrap().starts_with("Hello, x!"));
        assert_eq!(g.add(40, 2).unwrap(), Sum { sum: 42 });
        assert_err!(g.add(i64::MAX, 1), PluginError::Call { .. });
        assert_err!(g.boom(), PluginError::Panicked(_));
        assert!(g.shout("y").unwrap().starts_with("HELLO, Y!"));
        let err = g.missing(vec![1]).unwrap_err().to_string();
        assert!(err.contains("unknown method `missing`") && err.contains("available: greet, add"), "{err}");
    }
    assert!(m.bind("nope").is_err());

    // The same arguments by name, positionally, and with a typo.
    assert_eq!(text(m.call("typed", "add", br#"{"a":1,"b":2}"#)), r#"{"sum":3}"#);
    assert_eq!(text(m.call("typed", "add", b"[1,2]")), r#"{"sum":3}"#);
    assert_err!(m.call("typed", "add", br#"{"a":1,"b":2,"c":3}"#), PluginError::Call { .. });
    assert_err!(m.call("typed", "add", b"[1,2,3]"), PluginError::Call { .. });

    let methods = m.describe("typed").unwrap();
    let add = methods.iter().find(|mi| mi.name == "add").unwrap();
    assert_eq!(add.params, ["a", "b"]);
    assert!(methods.iter().any(|mi| mi.name == "read_file"));

    m.load(wasm("typed-wasm", "wasmi")).unwrap();
    let mut g = GreeterClient::new(m.bind("typed-wasm").unwrap());
    assert!(g.greet("w").unwrap().starts_with("Hello, w! (call #1 inside wasm)"));
    assert!(m.describe("typed-wasm").unwrap().iter().any(|mi| mi.name == "loop"));
}

/// Typed calls that misbehave on the other side of a process boundary.
#[plugwright::interface(client = Risky)]
trait Misbehave {
    fn crash(&mut self) -> plugwright::Result<()>;
    fn sleep(&mut self, ms: u64) -> plugwright::Result<String>;
    fn read_file(&mut self, path: &str) -> plugwright::Result<String>;
}

#[test]
fn typed_calls_across_isolation_crashes_and_static_libraries() {
    let mut m = manager();
    m.load(native("typed-iso", Isolation::Process)).unwrap();
    {
        let mut g = GreeterClient::new(m.bind("typed-iso").unwrap());
        assert_eq!(g.add(20, 22).unwrap(), Sum { sum: 42 });
        assert!(g.greet("a").unwrap().contains("call #1"));
    }
    {
        let mut r = Risky::new(m.bind("typed-iso").unwrap());
        assert_err!(r.crash(), PluginError::Crashed(_));
        assert_err!(r.sleep(10_000), PluginError::Timeout(_));
        assert_err!(r.read_file("/definitely/not/here"), PluginError::Call { .. });
    }
    let mut g = GreeterClient::new(m.bind("typed-iso").unwrap());
    assert!(g.greet("b").unwrap().contains("call #1"), "restarted runner, fresh state");

    // Through the plugin object itself instead of the manager.
    let plugin = m.get_mut("typed-iso").unwrap();
    assert!(GreeterClient::new(plugin).greet("c").unwrap().starts_with("Hello, c!"));

    let mut s = PluginSpec::new("typed-static", PluginKind::Native, Source::Path("static-hello".into()));
    s.isolation = Some(Isolation::InProcess);
    m.load(s).unwrap();
    let methods = m.describe("typed-static").unwrap();
    assert_eq!(methods.len(), 1);
    assert_eq!((methods[0].name.as_str(), methods[0].params.as_slice()), ("greet", &["name".to_owned()][..]));
    assert_eq!(
        GreeterClient::new(m.bind("typed-static").unwrap()).greet("z").unwrap(),
        "Hello from a static library, z!"
    );
    let err = GreeterClient::new(m.bind("typed-static").unwrap()).add(1, 2).unwrap_err().to_string();
    assert!(err.contains("unknown method `add` (available: greet)"), "{err}");
}

/// `NativeLoader`: build and open cdylib and staticlib plugins in-process, in
/// place, without a manifest or lockfile.
#[test]
fn native_loader_builds_and_opens_dynamic_and_static_libraries() {
    use plugwright::{Backend, NativeLoader, Plugin};
    let loader = NativeLoader::new(Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e")).unwrap();
    let examples = workspace().join("examples");

    let mut dynamic = loader.load(examples.join("native-hello")).unwrap();
    assert_eq!(dynamic.info().backend, Backend::Native);
    assert!(String::from_utf8(dynamic.call("greet", b"d").unwrap()).unwrap().starts_with("Hello, d!"));
    let built = loader.build(examples.join("native-hello")).unwrap();
    assert!(built.cached);
    let mut reopened = loader.open(&built.path).unwrap();
    assert!(String::from_utf8(reopened.call("greet", b"r").unwrap()).unwrap().contains("call #1"), "own instance");

    let mut stat = loader.load(examples.join("static-hello")).unwrap();
    assert_eq!(stat.call("greet", b"s").unwrap(), b"Hello from a static library, s!");
    // The archive itself opens too: relinked once, then served from the cache.
    let builder = Builder::new(loader.cache().clone(), BuildOptions::default());
    let archive = builder
        .build(&BuildRequest {
            id: "static-hello".into(),
            crate_dir: examples.join("static-hello"),
            target: Target::Native,
            features: vec![],
            no_default_features: false,
            source_label: "test".into(),
            in_process: true,
            crate_type: None,
        })
        .unwrap();
    assert_eq!(std::fs::read(&archive.path).unwrap()[..8], *b"!<arch>\n");
    let mut from_archive = loader.open(&archive.path).unwrap();
    assert_eq!(from_archive.call("greet", b"a").unwrap(), b"Hello from a static library, a!");
}

/// `WasmLoader`: build, compile, run; the compiled module is shared within the
/// process and its machine code kept for the next one. `Loader` opens the same
/// files as the specialised loaders, picking by magic bytes.
#[test]
fn wasm_loader_keeps_compiled_code_and_loader_picks_the_backend() {
    use plugwright::{Backend, Loader, Plugin, WasmLoader, WasmiEngine};
    use std::sync::Arc;
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e");
    let examples = workspace().join("examples");
    let loader = WasmLoader::new(&cache).unwrap();

    let mut a = loader.load(examples.join("wasm-hello")).unwrap();
    assert_eq!(a.info().engine.as_deref(), Some("wasmtime"));
    assert!(text(a.call("greet", b"a")).starts_with("Hello, a! (call #1 inside wasm)"));
    let built = loader.build(examples.join("wasm-hello")).unwrap();
    assert!(built.cached);
    let mut b = loader.open(&built.path).unwrap();
    assert!(text(b.call("greet", b"b")).contains("call #1"), "own instance, shared module");
    let kept = std::fs::read_dir(loader.cache().compiled_dir().join("wasmtime")).unwrap();
    assert!(kept.flatten().any(|f| f.file_name().to_string_lossy().starts_with(&built.sha256)));

    let mut on_wasmi =
        WasmLoader::new(&cache).unwrap().with_engine(Arc::new(WasmiEngine::new())).open(&built.path).unwrap();
    assert_eq!(on_wasmi.info().engine.as_deref(), Some("wasmi"));
    assert!(text(on_wasmi.call("greet", b"i")).starts_with("Hello, i!"));

    let any = Loader::new(&cache).unwrap();
    let native = any.build(examples.join("native-hello")).unwrap();
    for (path, backend) in [(&built.path, Backend::Wasm), (&native.path, Backend::Native)] {
        let mut p = any.open(path).unwrap();
        assert_eq!(p.info().backend, backend);
        assert!(text(p.call("greet", b"x")).starts_with("Hello, x!"));
    }
    assert_eq!(any.load_wasm(examples.join("wasm-hello")).unwrap().info().backend, Backend::Wasm);
}

/// A Wasm plugin outlives the loader and engine that opened it: wasmtime's
/// epoch ticker belongs to the compiled module too, so the wall-clock timeout
/// still fires.
#[test]
fn wasm_timeout_still_fires_after_the_loader_is_gone() {
    use plugwright::{Plugin, WasmLimits, WasmLoader};
    let limits = WasmLimits { fuel: None, timeout: Some(Duration::from_millis(100)), ..WasmLimits::default() };
    let loader = WasmLoader::new(Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e")).unwrap();
    let mut plugin = loader.with_limits(limits).load(workspace().join("examples/wasm-hello")).unwrap();
    assert_eq!(plugin.info().engine.as_deref(), Some("wasmtime"));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(plugin.call("loop", b"").map(drop)));
    let r = rx.recv_timeout(Duration::from_secs(30)).expect("the guest loop was never interrupted");
    assert_err!(r, PluginError::Wasm(WasmError::Timeout));
}

/// Two live loads of one file never share an image: each gets its own SDK
/// globals, so each instance's logs reach its own host callback.
#[test]
fn second_load_of_the_same_file_gets_its_own_image() {
    use plugwright::{NativePlugin, Placement};
    use std::sync::{Arc, Mutex};
    let mut m = manager();
    let built = m.prepare(&native("img", Isolation::InProcess)).unwrap();
    let sink = |lines: &Arc<Mutex<Vec<String>>>| {
        let lines = Arc::clone(lines);
        Arc::new(move |_: u32, msg: &str| lines.lock().unwrap().push(msg.to_owned())) as plugwright::logging::LogSink
    };
    let (a_logs, b_logs) = (Arc::default(), Arc::default());
    let a = NativePlugin::load(
        "a",
        &built.path,
        Placement::InPlace { copies: None },
        br#"{"greeting":"A"}"#,
        sink(&a_logs),
    )
    .unwrap();
    let b = NativePlugin::load(
        "b",
        &built.path,
        Placement::InPlace { copies: None },
        br#"{"greeting":"B"}"#,
        sink(&b_logs),
    )
    .unwrap();
    for (logs, g) in [(&a_logs, "\"A\""), (&b_logs, "\"B\"")] {
        let logs = logs.lock().unwrap();
        assert!(logs.len() == 1 && logs[0].contains(g), "{logs:?}");
    }
    drop((a, b));
}

/// The wasm-hello module, built once, plus a custom section that changes its
/// bytes (and so its hash) without changing what it does.
fn hello_wasm(variant: u8) -> Vec<u8> {
    let artifact = manager().prepare(&wasm("watch-src", "wasmi")).unwrap();
    let mut bytes = std::fs::read(artifact.path).unwrap();
    if variant > 0 {
        // Custom section: id 0, size 3, name "t", one payload byte.
        bytes.extend([0, 3, 1, b't', variant]);
    }
    bytes
}

/// Several writes in a row, as an editor's save does.
fn save(path: &Path, bytes: &[u8]) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    for chunk in bytes.chunks(bytes.len().div_ceil(3)) {
        f.write_all(chunk).unwrap();
        f.flush().unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn watched(dir: &Path) -> (PluginManager, PathBuf) {
    let file = dir.join("hello.wasm");
    std::fs::write(&file, hello_wasm(0)).unwrap();
    let mut m = manager();
    let mut s = PluginSpec::new("hello", PluginKind::Wasm, Source::Artifact(file.clone()));
    s.engine = Some("wasmi".into());
    m.load(s).unwrap();
    // Events from before a watcher exists must not reach it.
    std::thread::sleep(Duration::from_millis(300));
    (m, file)
}

fn sha(m: &PluginManager) -> String {
    m.list().next().unwrap().2.sha256.clone()
}

const WATCH: WatchOptions = WatchOptions { debounce: Duration::from_millis(150) };

#[test]
fn watcher_reloads_once_per_save_and_keeps_the_old_instance_on_a_broken_change() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut m, file) = watched(tmp.path());
    let first = sha(&m);
    assert!(text(m.call("hello", "greet", b"w")).contains("call #1"));

    // Off by default: without a watcher a change is not picked up.
    save(&file, &hello_wasm(1));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(sha(&m), first);
    assert!(text(m.call("hello", "greet", b"w")).contains("call #2"), "same instance");
    save(&file, &hello_wasm(0));
    std::thread::sleep(Duration::from_millis(300));

    let mut w = m.watch(&WATCH).unwrap();
    // Same bytes as loaded: whatever the OS reports, nothing is swapped.
    w.wait(Duration::from_millis(500));
    assert!(m.reload_changed(&mut w).is_empty());

    save(&file, &hello_wasm(2));
    assert!(w.wait(Duration::from_secs(10)), "the save must be seen");
    let res = m.reload_changed(&mut w);
    assert_eq!(res.len(), 1, "{res:?}");
    assert!(matches!(&res[0], (id, Ok(())) if id == "hello"), "{res:?}");
    let second = sha(&m);
    assert_ne!(second, first);
    assert!(text(m.call("hello", "greet", b"w")).contains("call #1"), "fresh instance");
    // Late events of the same save never reload a second time.
    w.wait(Duration::from_millis(600));
    assert!(m.reload_changed(&mut w).is_empty(), "one reload per save");
    assert!(text(m.call("hello", "greet", b"w")).contains("call #2"));

    save(&file, b"not wasm at all");
    assert!(w.wait(Duration::from_secs(10)));
    let res = m.reload_changed(&mut w);
    assert!(matches!(&res[..], [(id, Err(PluginError::NotAPlugin { .. }))] if id == "hello"), "{res:?}");
    assert_eq!(sha(&m), second, "the old instance stays");
    assert!(text(m.call("hello", "greet", b"w")).contains("call #3"), "and keeps serving");
    w.stop();
}

#[test]
fn reloader_thread_reloads_in_the_background_until_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, file) = watched(tmp.path());
    let first = sha(&m);
    let m = Arc::new(Mutex::new(m));
    let w = m.lock().unwrap().watch(&WATCH).unwrap();
    let (tx, rx) = mpsc::channel();
    let reloader = w.spawn(m.clone(), move |id, r| tx.send((id.to_owned(), r.is_ok())).unwrap()).unwrap();

    save(&file, &hello_wasm(3));
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), ("hello".to_owned(), true));
    let second = sha(&m.lock().unwrap());
    assert_ne!(second, first);

    reloader.stop();
    save(&file, &hello_wasm(4));
    std::thread::sleep(Duration::from_millis(600));
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)), "the thread is gone");
    assert_eq!(sha(&m.lock().unwrap()), second, "a stopped watcher reloads nothing");
}
