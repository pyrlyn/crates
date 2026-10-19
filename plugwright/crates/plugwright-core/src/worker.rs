//! One loaded plugin behind a thread of its own. [`Plugin::call`] takes
//! `&mut self`, so calls into one plugin run one at a time while different
//! plugins run in parallel, and a slow plugin never holds a thread of the
//! application's. Calls wait in two lanes, control before events, and a
//! deadline thread fires the plugin's [`Plugin::interrupter`] when a call
//! runs past its deadline.

use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{PluginError, Result};
use crate::plugin::{Interrupt, Plugin, PluginInfo, Release};

/// Control lane depth: calls something in the application waits on.
pub const CONTROL_DEPTH: usize = 16;
/// Event lane depth.
pub const EVENT_DEPTH: usize = 256;

/// The queue a call waits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Served first: something in the application is waiting on the answer.
    Control,
    /// Served only when the control queue is empty.
    Event,
}

type Reply = Result<Option<Vec<u8>>>;

struct Job {
    method: String,
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
    /// Set when the interrupt fired for the call now running, so its error
    /// is reported as the timeout it is, whatever text the backend gives.
    fired: bool,
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

/// A plugin running on its own thread. Dropping it interrupts the running
/// call and stops its threads; [`Worker::close`] also reports the unload.
pub struct Worker {
    shared: Arc<Shared>,
    interrupt: Option<Interrupt>,
    info: PluginInfo,
    call_cap: Duration,
    server: Option<JoinHandle<Release>>,
    watcher: Option<JoinHandle<()>>,
    /// Set by [`Self::stop`]. The threads keep running underneath, so holders
    /// of a shared worker can learn it was retired without racing its
    /// teardown, which stays with `Drop`.
    stopped: AtomicBool,
}

impl Worker {
    /// Moves `plugin` onto a thread named `name`. When the plugin has an
    /// [`Plugin::interrupter`], a second thread, `<name>-deadline`, enforces
    /// each call's deadline, never longer than `call_cap`. Without one the
    /// backend's own limits are the only bound on a call.
    pub fn spawn(plugin: Box<dyn Plugin>, name: &str, call_cap: Duration) -> Result<Self> {
        let interrupt = plugin.interrupter();
        let shared = Arc::new(Shared::default());
        let mut worker = Self {
            shared: shared.clone(),
            interrupt: interrupt.clone(),
            info: plugin.info().clone(),
            call_cap,
            server: None,
            watcher: None,
            stopped: AtomicBool::new(false),
        };
        if let Some(interrupt) = interrupt {
            let dog = shared.clone();
            worker.watcher = Some(start(format!("{name}-deadline"), move || watch(&dog, &interrupt))?);
        }
        worker.server = Some(start(name.to_owned(), move || serve(plugin, &shared))?);
        Ok(worker)
    }

    /// What the plugin reported about itself when it was spawned.
    pub fn info(&self) -> &PluginInfo {
        &self.info
    }

    /// The longest any one call may run.
    pub fn call_cap(&self) -> Duration {
        self.call_cap
    }

    /// Queues `method` on `lane` and waits for its answer. `deadline` (never
    /// more than [`Self::call_cap`]) starts when the worker begins the call,
    /// not when it is queued. A method the plugin does not have answers
    /// `Ok(None)`; a full lane answers [`PluginError::Busy`] at once.
    pub fn call(&self, lane: Lane, method: &str, input: Vec<u8>, deadline: Duration) -> Result<Option<Vec<u8>>> {
        let (reply, answer) = sync_channel(1);
        let job = Job { method: method.to_owned(), input, deadline: deadline.min(self.call_cap), reply };
        {
            let mut q = lock(&self.shared.queues);
            if q.closed {
                return Err(PluginError::Stopped);
            }
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
        answer.recv().map_err(|_| PluginError::Stopped)?
    }

    /// [`Self::call`] with JSON in and out.
    pub fn call_json<I: Serialize + ?Sized, O: DeserializeOwned>(
        &self,
        lane: Lane,
        method: &str,
        input: &I,
        deadline: Duration,
    ) -> Result<Option<O>> {
        match self.call(lane, method, serde_json::to_vec(input)?, deadline)? {
            Some(out) => Ok(Some(serde_json::from_slice(&out)?)),
            None => Ok(None),
        }
    }

    /// Flags this worker retired so [`Self::is_stopped`] answers `true` from
    /// here on. Idempotent; calls keep being served until the last holder
    /// drops the worker.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// Whether [`Self::stop`] has been called.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// Stops the threads and reports what [`Plugin::close`] could verify.
    pub fn close(mut self) -> Release {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Release {
        lock(&self.shared.queues).closed = true;
        lock(&self.shared.watch).closed = true;
        self.shared.work.notify_all();
        self.shared.tick.notify_all();
        // A call still running would otherwise hold the join for up to
        // `call_cap`.
        if let Some(interrupt) = &self.interrupt {
            interrupt();
        }
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        // A server that died could not close its plugin, so nothing about the
        // unload is known.
        self.server.take().map_or(Release::Freed, |server| server.join().unwrap_or(Release::StillMapped))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.server.is_some() || self.watcher.is_some() {
            self.shutdown();
        }
    }
}

fn start<T: Send + 'static>(name: String, f: impl FnOnce() -> T + Send + 'static) -> Result<JoinHandle<T>> {
    thread::Builder::new()
        .name(name)
        .spawn(f)
        .map_err(|e| PluginError::Load(format!("cannot start the plugin worker: {e}")))
}

fn serve(mut plugin: Box<dyn Plugin>, shared: &Shared) -> Release {
    while let Some(job) = next(shared) {
        let reply = if plugin.has_method(&job.method) {
            arm(shared, Some(Instant::now() + job.deadline));
            let out = panic::catch_unwind(AssertUnwindSafe(|| plugin.call(&job.method, &job.input)));
            let fired = arm(shared, None);
            match out {
                Ok(Err(_)) if fired => Err(PluginError::Timeout(job.deadline)),
                Ok(out) => out.map(Some),
                Err(payload) => {
                    // The plugin's state is unknown after a panic: answer this
                    // call, then refuse the rest instead of serving on it.
                    let _ = job.reply.send(Err(PluginError::Panicked(panic_message(&*payload))));
                    retire(shared);
                    return Release::StillMapped;
                }
            }
        } else {
            Ok(None)
        };
        // The caller may have gone; its answer is simply dropped.
        let _ = job.reply.send(reply);
    }
    plugin.close()
}

/// The next job, or `None` once the worker is closed. Jobs still queued then
/// are dropped, and their callers see [`PluginError::Stopped`].
fn next(shared: &Shared) -> Option<Job> {
    let mut q = lock(&shared.queues);
    loop {
        if q.closed {
            return None;
        }
        if let Some(job) = q.pop() {
            return Some(job);
        }
        q = shared.work.wait(q).unwrap_or_else(PoisonError::into_inner);
    }
}

fn retire(shared: &Shared) {
    let mut q = lock(&shared.queues);
    q.closed = true;
    q.control.clear();
    q.events.clear();
}

/// Sets the running call's deadline (`None` once it returns) and answers
/// whether the interrupt fired for the call being disarmed.
fn arm(shared: &Shared, deadline: Option<Instant>) -> bool {
    let mut w = lock(&shared.watch);
    w.deadline = deadline;
    let fired = std::mem::take(&mut w.fired);
    drop(w);
    shared.tick.notify_one();
    fired
}

fn watch(shared: &Shared, interrupt: &Interrupt) {
    let mut w = lock(&shared.watch);
    while !w.closed {
        let Some(at) = w.deadline else {
            w = shared.tick.wait(w).unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        let now = Instant::now();
        if now >= at {
            // Fired under the lock: the worker cannot clear the deadline and
            // start the next call first, so the interrupt can only reach the
            // call it was armed for, or none.
            interrupt();
            w.deadline = None;
            w.fired = true;
        } else {
            w = shared.tick.wait_timeout(w, at - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, Sender, channel};

    use super::*;
    use crate::error::WasmError;
    use crate::plugin::Backend;

    /// `echo` returns its input, `spin` runs until interrupted, `hold`
    /// reports it started and then waits for the gate, `name` answers the
    /// thread it runs on, `panic` panics. Nothing else exists.
    struct Fake {
        info: PluginInfo,
        stop: Arc<AtomicBool>,
        interruptible: bool,
        started: Sender<()>,
        gate: Receiver<()>,
    }

    struct Handles {
        started: Receiver<()>,
        gate: Sender<()>,
    }

    fn fake(interruptible: bool) -> (Box<dyn Plugin>, Handles) {
        let (started, started_rx) = channel();
        let (gate_tx, gate) = channel();
        let info = PluginInfo {
            name: "fake".into(),
            version: semver::Version::new(1, 0, 0),
            sdk_version: semver::Version::new(0, 1, 0),
            backend: Backend::Wasm,
            engine: None,
            sandbox: None,
        };
        let plugin = Fake { info, stop: Arc::default(), interruptible, started, gate };
        (Box::new(plugin), Handles { started: started_rx, gate: gate_tx })
    }

    impl Plugin for Fake {
        fn info(&self) -> &PluginInfo {
            &self.info
        }

        fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
            match method {
                "echo" => Ok(input.to_vec()),
                "spin" => {
                    while !self.stop.swap(false, Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(WasmError::Trap("interrupted".into()).into())
                }
                "hold" => {
                    let _ = self.started.send(());
                    let _ = self.gate.recv();
                    Ok(Vec::new())
                }
                "name" => Ok(thread::current().name().unwrap_or_default().as_bytes().to_vec()),
                "panic" => panic!("boom"),
                other => Err(PluginError::Call { method: other.into(), message: "unknown".into() }),
            }
        }

        fn has_method(&self, method: &str) -> bool {
            matches!(method, "echo" | "spin" | "hold" | "name" | "panic")
        }

        fn interrupter(&self) -> Option<Interrupt> {
            let stop = self.stop.clone();
            self.interruptible.then(|| Arc::new(move || stop.store(true, Ordering::SeqCst)) as Interrupt)
        }

        fn close(self: Box<Self>) -> Release {
            Release::Unmapped
        }
    }

    const CAP: Duration = Duration::from_secs(30);

    fn echo(worker: &Worker) -> Option<Vec<u8>> {
        worker.call(Lane::Control, "echo", b"hi".to_vec(), CAP).expect("echo")
    }

    #[test]
    fn calls_round_trip_on_the_named_thread() {
        let (plugin, _h) = fake(true);
        let worker = Worker::spawn(plugin, "pw-fake", CAP).unwrap();
        assert_eq!(echo(&worker).as_deref(), Some(&b"hi"[..]));
        let name = worker.call(Lane::Event, "name", Vec::new(), CAP).unwrap().unwrap();
        assert_eq!(name, b"pw-fake");
        assert_eq!(worker.info().name, "fake");
        let out: Option<String> = worker.call_json(Lane::Control, "echo", "json", CAP).unwrap();
        assert_eq!(out.as_deref(), Some("json"));
    }

    #[test]
    fn absent_method_is_none_not_error() {
        let (plugin, _h) = fake(true);
        let worker = Worker::spawn(plugin, "pw-fake", CAP).unwrap();
        assert_eq!(worker.call(Lane::Control, "render", Vec::new(), CAP).unwrap(), None);
    }

    #[test]
    fn runaway_call_is_interrupted_at_its_deadline() {
        let (plugin, _h) = fake(true);
        let worker = Worker::spawn(plugin, "pw-fake", CAP).unwrap();
        let started = Instant::now();
        let err = worker.call(Lane::Event, "spin", Vec::new(), Duration::from_millis(100)).unwrap_err();
        assert!(matches!(err, PluginError::Timeout(d) if d == Duration::from_millis(100)), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        // The worker survives the interrupt and serves the next call.
        assert!(echo(&worker).is_some());
    }

    #[test]
    fn deadline_never_exceeds_the_call_cap() {
        let (plugin, _h) = fake(true);
        let worker = Worker::spawn(plugin, "pw-fake", Duration::from_millis(50)).unwrap();
        assert_eq!(worker.call_cap(), Duration::from_millis(50));
        let err = worker.call(Lane::Control, "spin", Vec::new(), CAP).unwrap_err();
        assert!(matches!(err, PluginError::Timeout(d) if d == Duration::from_millis(50)), "{err:?}");
    }

    #[test]
    fn plugin_without_interrupter_gets_no_deadline_thread() {
        let (plugin, _h) = fake(false);
        let worker = Worker::spawn(plugin, "pw-fake", CAP).unwrap();
        assert!(worker.watcher.is_none());
        assert!(echo(&worker).is_some());
    }

    #[test]
    fn full_lane_is_busy() {
        let (plugin, h) = fake(true);
        let worker = Arc::new(Worker::spawn(plugin, "pw-fake", CAP).unwrap());
        let held = {
            let worker = worker.clone();
            thread::spawn(move || worker.call(Lane::Event, "hold", Vec::new(), CAP))
        };
        h.started.recv().unwrap();
        let queued: Vec<_> = (0..CONTROL_DEPTH)
            .map(|_| {
                let worker = worker.clone();
                thread::spawn(move || worker.call(Lane::Control, "echo", Vec::new(), CAP))
            })
            .collect();
        let waited = Instant::now();
        while lock(&worker.shared.queues).control.len() < CONTROL_DEPTH {
            assert!(waited.elapsed() < Duration::from_secs(10), "callers never queued");
            thread::sleep(Duration::from_millis(1));
        }
        let err = worker.call(Lane::Control, "echo", Vec::new(), CAP).unwrap_err();
        assert!(matches!(err, PluginError::Busy), "{err:?}");
        // The event lane is a separate queue.
        assert_eq!(lock(&worker.shared.queues).events.len(), 0);

        h.gate.send(()).unwrap();
        assert!(held.join().unwrap().is_ok());
        for caller in queued {
            assert!(caller.join().unwrap().is_ok());
        }
    }

    #[test]
    fn control_queue_is_served_before_events() {
        let job = |method: &str| Job {
            method: method.into(),
            input: Vec::new(),
            deadline: Duration::ZERO,
            reply: sync_channel(1).0,
        };
        let mut q = Queues::default();
        q.events.push_back(job("event"));
        q.control.push_back(job("control"));
        let order: Vec<String> = std::iter::from_fn(|| q.pop()).map(|j| j.method).collect();
        assert_eq!(order, ["control", "event"]);
    }

    #[test]
    fn panicking_plugin_answers_panicked_then_stopped() {
        let (plugin, _h) = fake(true);
        let worker = Worker::spawn(plugin, "pw-fake", CAP).unwrap();
        let err = worker.call(Lane::Control, "panic", Vec::new(), CAP).unwrap_err();
        assert!(matches!(&err, PluginError::Panicked(m) if m == "boom"), "{err:?}");
        let err = worker.call(Lane::Control, "echo", Vec::new(), CAP).unwrap_err();
        assert!(matches!(err, PluginError::Stopped), "{err:?}");
        assert_eq!(worker.close(), Release::StillMapped);
    }

    #[test]
    fn close_reports_the_plugins_release() {
        let (plugin, _h) = fake(true);
        let worker = Worker::spawn(plugin, "pw-fake", CAP).unwrap();
        assert!(echo(&worker).is_some());
        assert_eq!(worker.close(), Release::Unmapped);
    }

    #[test]
    fn stop_flags_without_closing_the_worker() {
        let (plugin, _h) = fake(true);
        let worker = Worker::spawn(plugin, "pw-fake", CAP).unwrap();
        assert!(!worker.is_stopped());
        worker.stop();
        assert!(worker.is_stopped());
        assert!(echo(&worker).is_some(), "worker still serves after stop");
    }
}
