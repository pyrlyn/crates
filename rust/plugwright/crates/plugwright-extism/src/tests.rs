use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde_json::{Value, json};

use super::*;

const INIT: &str = "plugin_init";
const OPTIONS: Options = Options::new(INIT, "test-plugin");

// The extism kernel imports every test module uses (the loader takes WAT
// text as well as binaries). WAT wants imports before functions.
const IMPORTS: &str = r#"
  (import "extism:host/env" "http_request" (func $http (param i64 i64) (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "input_load_u8" (func $load (param i64) (result i32)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))"#;

// `plugin_init` copies its input to its output.
const ECHO: &str = r#"
  (func $echo (result i32) (local $n i64) (local $off i64) (local $i i64)
    (local.set $n (call $input_length))
    (local.set $off (call $alloc (local.get $n)))
    (block $done (loop $copy
      (br_if $done (i64.ge_u (local.get $i) (local.get $n)))
      (call $store (i64.add (local.get $off) (local.get $i)) (call $load (local.get $i)))
      (local.set $i (i64.add (local.get $i) (i64.const 1)))
      (br $copy)))
    (call $output_set (local.get $off) (local.get $n))
    (i32.const 0))
  (export "plugin_init" (func $echo))"#;

const SPIN: &str = r#"(func (export "on_event") (result i32) (loop $l (br $l)) (i32.const 0))"#;

/// Grants nothing and counts the exports it is told about.
#[derive(Default)]
struct Env {
    entered: AtomicUsize,
}

impl HostEnv for Env {
    fn id(&self) -> &str {
        "t"
    }

    fn functions(self: &Arc<Self>) -> Vec<Function> {
        Vec::new()
    }

    fn enter(&self, export: &str) {
        if !export.is_empty() {
            self.entered.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn module(body: &str) -> Vec<u8> {
    format!("(module {IMPORTS} {body} {ECHO})").into_bytes()
}

fn start(body: &str, limits: &Limits) -> Worker {
    load(&module(body), limits, &OPTIONS, Arc::new(Env::default())).expect("module loads")
}

fn init(worker: &Worker) -> Option<Value> {
    worker.call_json(Lane::Control, INIT, &json!({ "greeting": "hi" }), worker.call_cap()).expect("init")
}

#[test]
fn wat_plugin_round_trips_json() {
    let env = Arc::new(Env::default());
    let worker = load(&module(""), &Limits::default(), &OPTIONS, env.clone()).expect("module loads");
    assert_eq!(init(&worker), Some(json!({ "greeting": "hi" })));
    assert_eq!(env.entered.load(Ordering::Relaxed), 1);
    assert_eq!(worker.info().name, "t");
    assert_eq!(worker.info().engine.as_deref(), Some("extism"));
}

#[test]
fn worker_threads_are_named_after_the_prefix_and_id() {
    struct Named;
    impl HostEnv for Named {
        fn id(&self) -> &str {
            "named"
        }
        fn functions(self: &Arc<Self>) -> Vec<Function> {
            Vec::new()
        }
        fn enter(&self, export: &str) {
            if !export.is_empty() {
                assert_eq!(std::thread::current().name(), Some("test-plugin-named"));
            }
        }
    }
    let worker = load(&module(""), &Limits::default(), &OPTIONS, Arc::new(Named)).expect("module loads");
    assert!(init(&worker).is_some());
}

#[test]
fn call_budget_defaults_and_overrides() {
    assert_eq!(start("", &Limits::default()).call_cap(), Duration::from_millis(200));
    let worker = start("", &Limits { call_ms: Some(1_500), ..Limits::default() });
    assert_eq!(worker.call_cap(), Duration::from_millis(1_500));
}

#[test]
fn runaway_call_is_cancelled_at_deadline() {
    let worker = start(SPIN, &Limits { call_ms: Some(30_000), ..Limits::default() });
    let started = Instant::now();
    let err = worker
        .call_json::<_, Value>(Lane::Event, "on_event", &json!({}), Duration::from_millis(100))
        .expect_err("a spin never returns");
    assert!(matches!(err, PluginError::Timeout(d) if d == Duration::from_millis(100)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    // The worker survives the cancel and serves the next call.
    assert!(init(&worker).is_some());
}

#[test]
fn call_budget_alone_stops_a_runaway_call() {
    // No worker, so no deadline thread: extism's own manifest timeout fires.
    let limits = Limits { call_ms: Some(100), ..Limits::default() };
    let mut plugin = ExtismPlugin::new(&module(SPIN), &limits, &OPTIONS, Arc::new(Env::default()), None).unwrap();
    let err = plugin.call("on_event", b"{}").expect_err("a spin never returns");
    assert!(matches!(err, PluginError::Timeout(d) if d == Duration::from_millis(100)), "{err:?}");
}

#[test]
fn memory_cap_traps_not_panics() {
    let worker = start(
        r#"(memory 1)
           (func (export "grow") (result i32)
             (drop (memory.grow (i32.const 1000))) (i32.const 0))"#,
        &Limits { memory_mib: Some(2), ..Limits::default() },
    );
    let err = worker
        .call_json::<_, Value>(Lane::Control, "grow", &json!({}), Duration::from_secs(5))
        .expect_err("growing past the cap traps");
    assert!(matches!(err, PluginError::Wasm(WasmError::MemoryLimit)), "{err:?}");
    assert!(init(&worker).is_some());
}

#[test]
fn missing_optional_export_is_absent_not_error() {
    let worker = start("", &Limits::default());
    let out: Option<Value> =
        worker.call_json(Lane::Control, "render", &json!({}), Duration::from_secs(1)).expect("absent export");
    assert_eq!(out, None);
    let no_init = load(b"(module)", &Limits::default(), &OPTIONS, Arc::new(Env::default()));
    assert!(matches!(no_init, Err(PluginError::Wasm(WasmError::MissingExport(e))) if e == INIT));
}

#[test]
fn unparsable_module_is_a_load_error() {
    let err = load(b"not wasm", &Limits::default(), &OPTIONS, Arc::new(Env::default()));
    assert!(matches!(err, Err(PluginError::Load(_))));
}

#[test]
fn http_request_is_compiled_out() {
    // Builds `{"url":"http://example.com"}` in kernel memory and calls
    // extism's built-in `http_request` with it.
    let worker = start(
        r#"(memory 1)
           (data (i32.const 0) "{\"url\":\"http://example.com\"}")
           (func (export "fetch") (result i32) (local $off i64) (local $i i64)
             (local.set $off (call $alloc (i64.const 28)))
             (block $done (loop $copy
               (br_if $done (i64.ge_u (local.get $i) (i64.const 28)))
               (call $store (i64.add (local.get $off) (local.get $i))
                 (i32.load8_u (i32.wrap_i64 (local.get $i))))
               (local.set $i (i64.add (local.get $i) (i64.const 1)))
               (br $copy)))
             (drop (call $http (local.get $off) (i64.const 0)))
             (i32.const 0))"#,
        &Limits::default(),
    );
    let err = worker
        .call_json::<_, Value>(Lane::Control, "fetch", &json!({}), Duration::from_secs(5))
        .expect_err("no http without the `http` feature");
    match err {
        PluginError::Wasm(WasmError::Trap(message)) => assert!(message.contains("not enabled"), "{message}"),
        other => panic!("expected a trap, got {other:?}"),
    }
}

#[test]
fn exception_handling_module_loads() {
    // Kotlin/Wasm emits `try_table`/`throw`; without extism's
    // `wasmtime-exceptions` the module fails to parse.
    let body = r#"(tag $e)
        (func (export "throws") (result i32)
          (block $caught (try_table (catch_all $caught) (throw $e)))
          (i32.const 0))"#;
    let worker = load(&module(body), &Limits::default(), &OPTIONS, Arc::new(Env::default()));
    assert!(worker.is_ok(), "exceptions proposal is on: {:?}", worker.err());
}

// Preview-1 imports a wasip1 guest (Go, Kotlin) links.
const WASI_IMPORTS: &str = r#"
  (import "wasi_snapshot_preview1" "environ_sizes_get" (func $env_sizes (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "args_sizes_get" (func $args_sizes (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))"#;

fn wasi_module(body: &str) -> Vec<u8> {
    module(&format!("{WASI_IMPORTS} (memory (export \"memory\") 1) {body}"))
}

fn with_wasi(body: &str, wasi: &Wasi) -> Result<Worker> {
    load_with_wasi(&wasi_module(body), &Limits::default(), &OPTIONS, Arc::new(Env::default()), wasi)
}

// The guests here set no output, so a clean call answers empty bytes; only a
// trap or a non-zero return is a failure.
fn run(worker: &Worker) -> Result<()> {
    worker.call(Lane::Control, "tool", Vec::new(), Duration::from_secs(5)).map(drop)
}

#[test]
fn wasi_is_off_unless_asked_for() {
    let body = r#"(func (export "tool") (result i32) (i32.const 0))"#;
    let off = load(&wasi_module(body), &Limits::default(), &OPTIONS, Arc::new(Env::default()));
    assert!(off.is_err(), "a WASI import must not link without WASI");
    assert!(with_wasi(body, &Wasi::default()).is_ok());
}

#[test]
fn wasi_ctx_has_no_env() {
    // The host process has PATH set; the guest must still see an empty
    // environment and no arguments. A non-zero return is a call error.
    assert!(std::env::var_os("PATH").is_some());
    let body = r#"(func (export "tool") (result i32)
          (drop (call $env_sizes (i32.const 0) (i32.const 4)))
          (drop (call $args_sizes (i32.const 8) (i32.const 12)))
          (i32.add (i32.load (i32.const 0)) (i32.load (i32.const 8))))"#;
    let worker = with_wasi(body, &Wasi::default()).expect("loads");
    run(&worker).expect("no environment variables and no arguments");
}

#[test]
fn read_only_preopen_refuses_a_create_and_a_writable_one_allows_it() {
    // `path_open("f.txt", O_CREAT)` on the first preopen (fd 3).
    let body = r#"(data (i32.const 16) "f.txt")
        (func (export "tool") (result i32)
          (call $path_open (i32.const 3) (i32.const 0) (i32.const 16) (i32.const 5)
            (i32.const 1) (i64.const 536870911) (i64.const 536870911) (i32.const 0) (i32.const 32)))"#;
    let dir = tempfile::tempdir().expect("tempdir");
    let host = dir.path().to_str().expect("utf-8").to_owned();
    let mount = |key: String| Wasi { paths: vec![(key, PathBuf::from("/ws"))] };
    let ro = with_wasi(body, &mount(format!("ro:{host}"))).expect("loads");
    assert!(run(&ro).is_err(), "a read-only mount creates nothing");
    assert!(!dir.path().join("f.txt").exists());
    let rw = with_wasi(body, &mount(host)).expect("loads");
    run(&rw).expect("a writable mount creates the file");
    assert!(dir.path().join("f.txt").exists());
}
