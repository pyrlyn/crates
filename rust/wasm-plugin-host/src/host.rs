// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: MIT OR Apache-2.0

//! One loaded plugin: a worker thread that owns the extism `Plugin` and serves
//! two queues, control before events, plus a watchdog thread that cancels a
//! call at its deadline through a `CancelHandle`. A thread per plugin because
//! `Plugin::call` takes `&mut self`: calls into one plugin run one at a time,
//! different plugins run in parallel, and a slow plugin never holds a thread
//! of the application's.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use extism::{CancelHandle, Function, Manifest, Plugin, PluginBuilder, Wasm};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::PluginError;

/// Control lane depth: calls something in the application waits on.
pub const CONTROL_DEPTH: usize = 16;
/// Event lane depth.
pub const EVENT_DEPTH: usize = 256;
const PAGES_PER_MIB: u32 = 16;

/// The application's side of one plugin: its host functions, and a hook told
/// which export is running so those functions can refuse what it may not do.
pub trait HostEnv: Send + Sync + 'static {
    /// The plugin's id; names its threads.
    fn id(&self) -> &str;

    /// The host functions the module may import. Called once, at load.
    fn functions(self: &Arc<Self>) -> Vec<Function>;

    /// Called with the export about to run, and with `""` once it returns.
    fn enter(&self, _export: &str) {}
}

/// What the application fixes for every plugin it loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The one export a module must have; loading fails without it.
    pub required_export: &'static str,
    /// Thread names are `<prefix>-<id>` and `<prefix>-<id>-deadline`.
    pub thread_prefix: &'static str,
    /// Linear memory when the plugin asks for none, in MiB.
    pub memory_mib: u32,
    /// The most linear memory a plugin may ask for, in MiB.
    pub max_memory_mib: u32,
    /// Per-call budget when the plugin asks for none, in milliseconds.
    pub call_ms: u32,
}

impl Options {
    /// Options with the default limits: 16 MiB of memory (at most 64) and a
    /// 200 ms call budget.
    pub const fn new(required_export: &'static str, thread_prefix: &'static str) -> Self {
        Self {
            required_export,
            thread_prefix,
            memory_mib: 16,
            max_memory_mib: 64,
            call_ms: 200,
        }
    }
}

/// What one plugin asks for. Absent means the [`Options`] default; memory is
/// clamped to [`Options::max_memory_mib`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    /// Linear memory in MiB.
    pub memory_mib: Option<u32>,
    /// Per-call budget in milliseconds; also the cap on every call's deadline.
    pub call_ms: Option<u32>,
}

/// The queue a call waits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Served first: something in the application is waiting on the answer.
    Control,
    /// Served only when the control queue is empty.
    Event,
}

type Reply = Result<Option<Vec<u8>>, PluginError>;

struct Job {
    export: String,
    input: Vec<u8>,
    deadline: Duration,
    reply: SyncSender<Reply>,
}

#[derive(Default)]
struct Queues {
    control: VecDeque<Job>,
    events: VecDeque<Job>,
    closed: bool,
}

impl Queues {
    fn pop(&mut self) -> Option<Job> {
        self.control.pop_front().or_else(|| self.events.pop_front())
    }
}

#[derive(Default)]
struct Watch {
    deadline: Option<Instant>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    queues: Mutex<Queues>,
    work: Condvar,
    watch: Mutex<Watch>,
    tick: Condvar,
}

// A poisoned lock only means another thread panicked mid-update; both
// structures stay consistent after every statement, so keep serving.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A loaded plugin. Dropping it cancels the running call and stops both threads.
pub struct PluginHost {
    shared: Arc<Shared>,
    cancel: CancelHandle,
    call_cap: Duration,
    threads: Vec<JoinHandle<()>>,
    /// Set by [`Self::stop`]. The threads keep running underneath, so holders
    /// of a shared host can learn it was retired without racing its teardown,
    /// which stays with `Drop`.
    stopped: Arc<AtomicBool>,
}

impl PluginHost {
    /// Compiles `wasm` (binary or WAT text) with no WASI and no compilation
    /// cache, under `limits` clamped by `options`, with `env`'s host functions.
    ///
    /// WASI stays off: wasmtime 43's WASI filesystem has a sandbox escape
    /// (RUSTSEC-2026-0269) until extism ships wasmtime >= 48. Each plugin gets
    /// its own wasmtime `Engine` because extism 1.30 builds one inside
    /// `CompiledPlugin::new` and offers no way to pass a shared one; that is
    /// safe from RUSTSEC-2026-0222 only while nothing here moves a wasmtime
    /// object from one plugin to another.
    pub fn load<E: HostEnv>(
        wasm: &[u8],
        limits: &Limits,
        options: &Options,
        env: Arc<E>,
    ) -> Result<Self, PluginError> {
        let mib = limits
            .memory_mib
            .unwrap_or(options.memory_mib)
            .min(options.max_memory_mib);
        let call_cap = Duration::from_millis(limits.call_ms.unwrap_or(options.call_ms).into());
        let manifest = Manifest::new([Wasm::data(wasm.to_vec())])
            .with_memory_max(mib * PAGES_PER_MIB)
            .with_timeout(call_cap);
        // The default cache writes under the user's system cache dir, which
        // belongs to no application in particular.
        let plugin = PluginBuilder::new(manifest)
            .with_wasi(false)
            .with_cache_disabled()
            .with_functions(env.functions())
            .build()
            .map_err(|e| PluginError::Load(format!("{e:#}")))?;
        if !plugin.function_exists(options.required_export) {
            return Err(PluginError::MissingExport(options.required_export));
        }
        let cancel = plugin.cancel_handle();
        let shared = Arc::new(Shared::default());
        let mut host = Self {
            shared: shared.clone(),
            cancel: cancel.clone(),
            call_cap,
            threads: Vec::new(),
            stopped: Arc::new(AtomicBool::new(false)),
        };
        let name = format!("{}-{}", options.thread_prefix, env.id());
        let dog = shared.clone();
        host.spawn(format!("{name}-deadline"), move || watch(&dog, &cancel))?;
        host.spawn(name, move || serve(plugin, &shared, &*env))?;
        Ok(host)
    }

    fn spawn(
        &mut self,
        name: String,
        f: impl FnOnce() + Send + 'static,
    ) -> Result<(), PluginError> {
        let handle = thread::Builder::new()
            .name(name)
            .spawn(f)
            .map_err(|e| PluginError::Load(e.to_string()))?;
        self.threads.push(handle);
        Ok(())
    }

    /// The plugin's per-call budget: [`Limits::call_ms`] or the default.
    pub fn call_cap(&self) -> Duration {
        self.call_cap
    }

    /// Calls `export` with JSON in and out, waiting at most `deadline`
    /// (never more than [`Self::call_cap`]) once the worker starts it.
    /// An optional export the guest does not have answers `Ok(None)`.
    pub fn call<I: Serialize, O: DeserializeOwned>(
        &self,
        lane: Lane,
        export: &str,
        input: &I,
        deadline: Duration,
    ) -> Result<Option<O>, PluginError> {
        let (reply, answer) = sync_channel(1);
        let job = Job {
            export: export.to_string(),
            input: serde_json::to_vec(input)?,
            deadline: deadline.min(self.call_cap),
            reply,
        };
        {
            let mut q = lock(&self.shared.queues);
            let (queue, depth) = match lane {
                Lane::Control => (&mut q.control, CONTROL_DEPTH),
                Lane::Event => (&mut q.events, EVENT_DEPTH),
            };
            if queue.len() >= depth {
                return Err(PluginError::Busy);
            }
            queue.push_back(job);
        }
        self.shared.work.notify_one();
        match answer.recv().map_err(|_| PluginError::Stopped)?? {
            Some(out) => Ok(Some(serde_json::from_slice(&out)?)),
            None => Ok(None),
        }
    }

    /// Flags this instance retired so [`Self::is_stopped`] answers `true` from
    /// here on. Idempotent; the queues and threads keep serving until the
    /// last holder drops the host.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// Whether [`Self::stop`] has been called.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        lock(&self.shared.queues).closed = true;
        lock(&self.shared.watch).closed = true;
        self.shared.work.notify_all();
        self.shared.tick.notify_all();
        // A call still running would otherwise hold the join for up to
        // `call_ms`; the timer ignores a cancel when nothing runs.
        let _ = self.cancel.cancel();
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}

fn serve<E: HostEnv>(mut plugin: Plugin, shared: &Shared, env: &E) {
    loop {
        let job = {
            let mut q = lock(&shared.queues);
            loop {
                if q.closed {
                    return;
                }
                if let Some(job) = q.pop() {
                    break job;
                }
                q = shared.work.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let reply = if plugin.function_exists(&job.export) {
            env.enter(&job.export);
            set_deadline(shared, Some(Instant::now() + job.deadline));
            let out = plugin.call::<&[u8], Vec<u8>>(&job.export, &job.input);
            set_deadline(shared, None);
            env.enter("");
            out.map(Some)
                .map_err(|e| PluginError::from_call(&job.export, &e))
        } else {
            Ok(None)
        };
        // The caller may have gone; its answer is simply dropped.
        let _ = job.reply.send(reply);
    }
}

fn set_deadline(shared: &Shared, deadline: Option<Instant>) {
    lock(&shared.watch).deadline = deadline;
    shared.tick.notify_one();
}

fn watch(shared: &Shared, cancel: &CancelHandle) {
    let mut w = lock(&shared.watch);
    while !w.closed {
        let Some(at) = w.deadline else {
            w = shared.tick.wait(w).unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        let now = Instant::now();
        if now >= at {
            // Sent under the lock: the worker cannot clear the deadline and
            // start the next call before this cancel is queued, so it can
            // only reach the call it was armed for, or none.
            let _ = cancel.cancel();
            w.deadline = None;
        } else {
            w = shared
                .tick
                .wait_timeout(w, at - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::atomic::AtomicUsize;

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

    fn load(body: &str, limits: &Limits) -> PluginHost {
        PluginHost::load(&module(body), limits, &OPTIONS, Arc::new(Env::default()))
            .expect("module loads")
    }

    fn init(host: &PluginHost) -> Option<Value> {
        host.call(
            Lane::Control,
            INIT,
            &json!({ "greeting": "hi" }),
            host.call_cap(),
        )
        .expect("init")
    }

    #[test]
    fn wat_plugin_round_trips_json() {
        let env = Arc::new(Env::default());
        let host = PluginHost::load(&module(""), &Limits::default(), &OPTIONS, env.clone())
            .expect("module loads");
        assert_eq!(init(&host), Some(json!({ "greeting": "hi" })));
        assert_eq!(env.entered.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn call_budget_defaults_and_overrides() {
        let host = load("", &Limits::default());
        assert_eq!(host.call_cap(), Duration::from_millis(200));
        let host = load(
            "",
            &Limits {
                call_ms: Some(1_500),
                ..Limits::default()
            },
        );
        assert_eq!(host.call_cap(), Duration::from_millis(1_500));
    }

    #[test]
    fn runaway_call_is_cancelled_at_deadline() {
        let limits = Limits {
            call_ms: Some(30_000),
            ..Limits::default()
        };
        let host = load(
            r#"(func (export "on_event") (result i32) (loop $l (br $l)) (i32.const 0))"#,
            &limits,
        );
        let started = Instant::now();
        let err = host
            .call::<_, Value>(
                Lane::Event,
                "on_event",
                &json!({}),
                Duration::from_millis(100),
            )
            .expect_err("a spin never returns");
        assert!(matches!(err, PluginError::Timeout { .. }), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        // The worker survives the cancel and serves the next call.
        assert!(init(&host).is_some());
    }

    #[test]
    fn memory_cap_traps_not_panics() {
        let limits = Limits {
            memory_mib: Some(2),
            ..Limits::default()
        };
        let host = load(
            r#"(memory 1)
               (func (export "grow") (result i32)
                 (drop (memory.grow (i32.const 1000))) (i32.const 0))"#,
            &limits,
        );
        let err = host
            .call::<_, Value>(Lane::Control, "grow", &json!({}), Duration::from_secs(5))
            .expect_err("growing past the cap traps");
        assert!(matches!(err, PluginError::OutOfMemory { .. }), "{err:?}");
        assert!(init(&host).is_some());
    }

    #[test]
    fn missing_optional_export_is_absent_not_error() {
        let host = load("", &Limits::default());
        let out: Option<Value> = host
            .call(Lane::Control, "render", &json!({}), Duration::from_secs(1))
            .expect("absent export is not an error");
        assert_eq!(out, None);
        let no_init = PluginHost::load(
            b"(module)",
            &Limits::default(),
            &OPTIONS,
            Arc::new(Env::default()),
        );
        assert!(matches!(no_init, Err(PluginError::MissingExport(INIT))));
    }

    #[test]
    fn unparsable_module_is_a_load_error() {
        let err = PluginHost::load(
            b"not wasm",
            &Limits::default(),
            &OPTIONS,
            Arc::new(Env::default()),
        );
        assert!(matches!(err, Err(PluginError::Load(_))));
    }

    #[test]
    fn stop_flags_is_stopped_without_closing_the_worker() {
        let host = load("", &Limits::default());
        assert!(!host.is_stopped());
        host.stop();
        assert!(host.is_stopped());
        assert!(init(&host).is_some(), "worker still serves after stop");
    }

    #[test]
    fn http_request_is_compiled_out() {
        // Builds `{"url":"http://example.com"}` in kernel memory and calls
        // extism's built-in `http_request` with it.
        let host = load(
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
        let err = host
            .call::<_, Value>(Lane::Control, "fetch", &json!({}), Duration::from_secs(5))
            .expect_err("no http without the `http` feature");
        match err {
            PluginError::Trap { message, .. } => {
                assert!(message.contains("not enabled"), "{message}")
            }
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
        let host = PluginHost::load(
            &module(body),
            &Limits::default(),
            &OPTIONS,
            Arc::new(Env::default()),
        );
        assert!(host.is_ok(), "exceptions proposal is on: {:?}", host.err());
    }

    #[test]
    fn control_queue_is_served_before_events() {
        let job = |export: &str| Job {
            export: export.into(),
            input: Vec::new(),
            deadline: Duration::ZERO,
            reply: sync_channel(1).0,
        };
        let mut q = Queues::default();
        q.events.push_back(job("event"));
        q.control.push_back(job("control"));
        let order: Vec<String> = std::iter::from_fn(|| q.pop()).map(|j| j.export).collect();
        assert_eq!(order, ["control", "event"]);
    }
}
