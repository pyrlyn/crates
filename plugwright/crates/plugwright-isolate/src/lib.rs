//! Out-of-process isolation for native plugins.
//!
//! The host spawns a *runner* (this same executable re-launched with
//! [`RUNNER_ARG`], or a dedicated binary such as `plugwright-runner`). The runner
//! loads the plugin with [`NativePlugin`] and serves requests over its
//! stdin/stdout using length-prefixed frames. A segfault, `abort()` or `exit()`
//! inside the plugin kills only the runner: the host sees EOF, reports
//! [`PluginError::Crashed`] and respawns the runner on the next call.
//!
//! Before loading the library the runner confines itself with the OS sandbox
//! from [`plugwright_sandbox`] (no writes, network or child processes by default),
//! so even the library's initialisers run confined.
//!
//! Frame: `u32 LE body length | u8 tag | (u32 LE field length | field bytes)*`.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use plugwright_core::error::{PluginError, Result};
use plugwright_core::logging::{self, LogSink};
use plugwright_core::plugin::{Backend, Plugin, PluginInfo, Release, check_sdk, parse_version};
use plugwright_native::{MAX_LOG_LINE, NativePlugin, Placement};
use plugwright_sandbox::{self as sandbox, SandboxPolicy};

/// First argument that switches an executable into runner mode.
pub const RUNNER_ARG: &str = "--plugwright-runner";

const MAGIC: &[u8] = b"PLUGWRT1";
const MAX_FRAME: usize = 64 << 20;

mod tag {
    pub const HELLO: u8 = 1;
    pub const LOAD: u8 = 2;
    pub const CALL: u8 = 3;
    pub const SHUTDOWN: u8 = 4;
    pub const READY: u8 = 10;
    pub const LOADED: u8 = 11;
    pub const OK: u8 = 12;
    pub const ERR: u8 = 13;
    pub const LOG: u8 = 14;
}

mod err_kind {
    pub const LOAD: u8 = 1;
    pub const CALL: u8 = 2;
    pub const PANIC: u8 = 3;
    pub const OTHER: u8 = 4;
}

type Frame = (u8, Vec<Vec<u8>>);

fn write_frame(w: &mut dyn Write, tag: u8, fields: &[&[u8]]) -> io::Result<()> {
    let body: usize = 1 + fields.iter().map(|f| 4 + f.len()).sum::<usize>();
    if body > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("frame of {body} bytes exceeds limit")));
    }
    let mut buf = Vec::with_capacity(4 + body);
    buf.extend_from_slice(&(body as u32).to_le_bytes());
    buf.push(tag);
    for f in fields {
        buf.extend_from_slice(&(f.len() as u32).to_le_bytes());
        buf.extend_from_slice(f);
    }
    w.write_all(&buf)?;
    w.flush()
}

/// Read one frame; `Ok(None)` on clean EOF before a frame starts.
fn read_frame(r: &mut dyn Read) -> io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    // Only EOF before the first byte is clean; a cut-off length prefix is a truncated frame.
    match r.read_exact(&mut len[..1]) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    r.read_exact(&mut len[1..])?;
    let len = u32::from_le_bytes(len) as usize;
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_owned());
    if len == 0 || len > MAX_FRAME {
        return Err(bad("frame length out of range"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let mut fields = Vec::new();
    let mut at = 1;
    while at < len {
        let Some(hdr) = body.get(at..at + 4) else { return Err(bad("truncated field header")) };
        let flen = u32::from_le_bytes(hdr.try_into().expect("4 bytes")) as usize;
        at += 4;
        let Some(field) = body.get(at..at.saturating_add(flen)) else { return Err(bad("truncated field")) };
        fields.push(field.to_vec());
        at += flen;
    }
    Ok(Some((body[0], fields)))
}

fn field(f: &[Vec<u8>], i: usize) -> Result<&[u8]> {
    f.get(i).map(Vec::as_slice).ok_or_else(|| PluginError::Protocol(format!("missing field {i}")))
}

fn field_str(f: &[Vec<u8>], i: usize) -> Result<String> {
    String::from_utf8(field(f, i)?.to_vec()).map_err(|_| PluginError::Protocol(format!("field {i} is not UTF-8")))
}

// ---------------------------------------------------------------------------
// Runner side
// ---------------------------------------------------------------------------

/// Call first thing in `main`. If the process was started as a plugwright
/// runner, serve plugin requests and exit; otherwise return immediately.
pub fn maybe_run_runner() {
    if std::env::args_os().nth(1).as_deref() == Some(OsStr::new(RUNNER_ARG)) {
        std::process::exit(run_runner());
    }
}

/// The runner's protocol channel, shared with the plugin's log sink.
type Out = Arc<Mutex<Box<dyn Write + Send>>>;

/// Serve the runner protocol on stdin/stdout. Returns the process exit code.
pub fn run_runner() -> i32 {
    let out: Out = match protocol_stdout() {
        Ok(w) => Arc::new(Mutex::new(w)),
        Err(e) => {
            eprintln!("plugwright-runner: cannot set up protocol channel: {e}");
            return 2;
        }
    };
    harden_process();
    serve(&mut io::stdin().lock(), &out)
}

/// The runner loop: 0 on EOF or SHUTDOWN, 3 on a protocol error, 4 when the
/// host stops reading.
fn serve(input: &mut dyn Read, out: &Out) -> i32 {
    let send = |tag: u8, fields: &[&[u8]]| -> bool {
        let mut w = out.lock().unwrap_or_else(|p| p.into_inner());
        write_frame(&mut **w, tag, fields).is_ok()
    };
    let mut plugin: Option<NativePlugin> = None;
    loop {
        let (t, f) = match read_frame(input) {
            Ok(Some(frame)) => frame,
            Ok(None) => return 0,
            Err(e) => {
                eprintln!("plugwright-runner: protocol error: {e}");
                return 3;
            }
        };
        let ok = match t {
            tag::HELLO if f.first().map(Vec::as_slice) == Some(MAGIC) => send(tag::READY, &[MAGIC]),
            tag::HELLO => {
                eprintln!("plugwright-runner: bad handshake");
                return 3;
            }
            tag::LOAD => {
                plugin = None; // unload any previous instance first
                match runner_load(&f, out) {
                    Ok((p, report)) => {
                        let i = p.info();
                        let (name, version, sdk) = (i.name.clone(), i.version.to_string(), i.sdk_version.to_string());
                        plugin = Some(p);
                        send(tag::LOADED, &[name.as_bytes(), version.as_bytes(), sdk.as_bytes(), report.as_bytes()])
                    }
                    Err(e) => send(tag::ERR, &[&[err_kind::LOAD], e.to_string().as_bytes()]),
                }
            }
            tag::CALL => {
                let reply = match (&mut plugin, field_str(&f, 0), field(&f, 1)) {
                    (None, ..) => Err((err_kind::OTHER, "no plugin loaded".to_owned())),
                    (_, Err(e), _) | (_, _, Err(e)) => Err((err_kind::OTHER, e.to_string())),
                    (Some(p), Ok(method), Ok(input)) => p.call(&method, input).map_err(|e| match e {
                        PluginError::Call { message, .. } => (err_kind::CALL, message),
                        PluginError::Panicked(m) => (err_kind::PANIC, m),
                        other => (err_kind::OTHER, other.to_string()),
                    }),
                };
                match reply {
                    Ok(bytes) => send(tag::OK, &[&bytes]),
                    Err((kind, msg)) => send(tag::ERR, &[&[kind], msg.as_bytes()]),
                }
            }
            tag::SHUTDOWN => {
                drop(plugin);
                return 0;
            }
            other => {
                eprintln!("plugwright-runner: unknown tag {other}");
                return 3;
            }
        };
        if !ok {
            return 4; // host went away
        }
    }
}

/// Sandbox the runner, then load the plugin in place. Returns the plugin and
/// a description of the enforced sandbox (JSON [`sandbox::Report`]).
fn runner_load(f: &[Vec<u8>], out: &Out) -> Result<(NativePlugin, String)> {
    let id = field_str(f, 0)?;
    let artifact = PathBuf::from(field_str(f, 1)?);
    let config = field(f, 2)?;
    let policy: SandboxPolicy = serde_json::from_slice(field(f, 3)?)?;
    // The runner is single-threaded here, which Landlock/seccomp rely on.
    let report = sandbox::apply(&policy, &artifact).map_err(|e| PluginError::Load(format!("sandbox: {e}")))?;
    let out = Arc::clone(out);
    let sink: LogSink = Arc::new(move |level, msg| {
        let mut w = out.lock().unwrap_or_else(|p| p.into_inner());
        let _ = write_frame(&mut **w, tag::LOG, &[&level.to_le_bytes(), msg.as_bytes()]);
    });
    // One process per load: no shadow copy needed, and the sandbox forbids writing one.
    let plugin = NativePlugin::load(&id, &artifact, Placement::InPlace { copies: None }, config, sink)?;
    Ok((plugin, serde_json::to_string(&report)?))
}

/// Move the protocol off fd 1 so `println!` inside a plugin lands on stderr
/// instead of corrupting frames.
#[cfg(unix)]
fn protocol_stdout() -> io::Result<Box<dyn Write + Send>> {
    use std::os::fd::FromRawFd;
    io::stdout().flush()?;
    // SAFETY: plain fd syscalls on descriptors this process owns.
    unsafe {
        let fd = libc::dup(1);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        if libc::dup2(2, 1) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Box::new(std::fs::File::from_raw_fd(fd)))
    }
}

/// Windows: keep a private duplicate of the stdout handle for the protocol,
/// then point both the Win32 standard handle (Rust `println!` looks it up on
/// every write) and the CRT's fd 1 (`printf`) at stderr.
#[cfg(windows)]
fn protocol_stdout() -> io::Result<Box<dyn Write + Send>> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    io::stdout().flush()?;
    // SAFETY: Win32 handle calls on this process's own standard handles; the
    // duplicate is owned by the returned `File`.
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let err = GetStdHandle(STD_ERROR_HANDLE);
        if out.is_null() || out == INVALID_HANDLE_VALUE {
            return Err(io::Error::other("runner has no stdout handle"));
        }
        let me = GetCurrentProcess();
        let mut dup: HANDLE = std::ptr::null_mut();
        // Not inheritable: nothing the plugin starts may write protocol frames.
        if DuplicateHandle(me, out, me, &mut dup, 0, 0, DUPLICATE_SAME_ACCESS) == 0 {
            return Err(io::Error::last_os_error());
        }
        if SetStdHandle(STD_OUTPUT_HANDLE, err) == 0 {
            return Err(io::Error::last_os_error());
        }
        // Fails harmlessly (-1) when the CRT has no fd 2, e.g. a GUI subsystem.
        libc::dup2(2, 1);
        Ok(Box::new(std::fs::File::from_raw_handle(dup)))
    }
}

/// Other platforms: no redirection. Stray plugin output is detected as a
/// protocol error by the host and handled like a crash.
#[cfg(not(any(unix, windows)))]
fn protocol_stdout() -> io::Result<Box<dyn Write + Send>> {
    Ok(Box::new(io::stdout()))
}

/// Disable core dumps: a crashing plugin must not spill host-provided data to disk.
fn harden_process() {
    #[cfg(unix)]
    unsafe {
        let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &none);
    }
}

// ---------------------------------------------------------------------------
// Host side
// ---------------------------------------------------------------------------

/// How to launch the runner process.
#[derive(Debug, Clone, Default)]
pub enum RunnerCommand {
    /// Re-execute the current binary; its `main` must call [`maybe_run_runner`].
    #[default]
    CurrentExe,
    /// A dedicated runner binary (e.g. `plugwright-runner`).
    Path(PathBuf),
}

/// Knobs for isolated plugins.
#[derive(Debug, Clone)]
pub struct IsolationOptions {
    pub runner: RunnerCommand,
    pub call_timeout: Duration,
    pub load_timeout: Duration,
    /// Restarts allowed inside `restart_window` before the plugin is disabled.
    pub max_restarts: u32,
    pub restart_window: Duration,
}

impl Default for IsolationOptions {
    fn default() -> Self {
        IsolationOptions {
            runner: RunnerCommand::CurrentExe,
            call_timeout: Duration::from_secs(30),
            load_timeout: Duration::from_secs(10),
            max_restarts: 3,
            restart_window: Duration::from_secs(60),
        }
    }
}

enum Msg {
    Frame(Frame),
    Closed(Option<String>),
}

struct RunnerProc {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Msg>,
}

/// A native plugin running in a child process. Implements [`Plugin`].
pub struct IsolatedPlugin {
    id: String,
    artifact: PathBuf,
    config: Vec<u8>,
    opts: IsolationOptions,
    sandbox: SandboxPolicy,
    sink: LogSink,
    proc: Option<RunnerProc>,
    info: PluginInfo,
    restarts: VecDeque<Instant>,
}

impl IsolatedPlugin {
    pub fn spawn(
        id: &str,
        artifact: &Path,
        config: &[u8],
        opts: IsolationOptions,
        sandbox: SandboxPolicy,
        sink: LogSink,
    ) -> Result<Self> {
        let mut this = IsolatedPlugin {
            id: id.to_owned(),
            artifact: artifact.to_owned(),
            config: config.to_vec(),
            opts,
            sandbox,
            sink,
            proc: None,
            info: PluginInfo {
                name: String::new(),
                version: semver::Version::new(0, 0, 0),
                sdk_version: semver::Version::new(0, 0, 0),
                backend: Backend::Isolated,
                engine: None,
                sandbox: None,
            },
            restarts: VecDeque::new(),
        };
        this.start()?;
        Ok(this)
    }

    fn start(&mut self) -> Result<()> {
        let started = Instant::now();
        let exe = match &self.opts.runner {
            RunnerCommand::CurrentExe => std::env::current_exe()
                .map_err(|e| PluginError::Load(format!("cannot locate current executable: {e}")))?,
            RunnerCommand::Path(p) => p.clone(),
        };
        let mut cmd = Command::new(&exe);
        cmd.arg(RUNNER_ARG).env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // Windows needs SystemRoot for the loader to resolve system DLLs.
        #[cfg(windows)]
        if let Some(v) = std::env::var_os("SystemRoot") {
            cmd.env("SystemRoot", v);
        }
        let mut child = cmd.spawn().map_err(|e| PluginError::Load(format!("cannot spawn runner {exe:?}: {e}")))?;
        let stdin = child.stdin.take().expect("piped");
        let mut stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");

        let (tx, rx) = mpsc::channel();
        let sink = Arc::clone(&self.sink);
        std::thread::Builder::new()
            .name(format!("plugwright-{}-out", self.id))
            .spawn(move || {
                loop {
                    match read_frame(&mut stdout) {
                        Ok(Some((tag::LOG, f))) if f.len() == 2 && f[0].len() == 4 => {
                            let level = u32::from_le_bytes(f[0][..4].try_into().expect("4 bytes"));
                            let msg = &f[1][..f[1].len().min(MAX_LOG_LINE)];
                            sink(level, &String::from_utf8_lossy(msg));
                        }
                        Ok(Some(frame)) => {
                            if tx.send(Msg::Frame(frame)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => {
                            let _ = tx.send(Msg::Closed(None));
                            return;
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Closed(Some(e.to_string())));
                            return;
                        }
                    }
                }
            })
            .map_err(|e| PluginError::Load(format!("cannot start reader thread: {e}")))?;
        let id = self.id.clone();
        let _ = std::thread::Builder::new().name(format!("plugwright-{}-err", self.id)).spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(io::Result::ok).filter(|l| !l.trim().is_empty()) {
                tracing::warn!(target: logging::GUEST_TARGET, plugin = %id, stream = "stderr", "{line}");
            }
        });

        self.proc = Some(RunnerProc { child, stdin, rx });
        let timeout = self.opts.load_timeout;
        let (t, f) = self.request(tag::HELLO, &[MAGIC], timeout)?;
        if t != tag::READY || f.first().map(Vec::as_slice) != Some(MAGIC) {
            self.kill();
            return Err(PluginError::Protocol("runner handshake failed".into()));
        }
        let artifact = self
            .artifact
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| PluginError::Load(format!("artifact path is not UTF-8: {}", self.artifact.display())))?;
        let config = self.config.clone();
        let policy = serde_json::to_vec(&self.sandbox)?;
        let (t, f) =
            self.request(tag::LOAD, &[self.id.clone().as_bytes(), artifact.as_bytes(), &config, &policy], timeout)?;
        match t {
            tag::LOADED => {
                self.info.name = field_str(&f, 0)?;
                self.info.version = parse_version("plugin version", &field_str(&f, 1)?)?;
                self.info.sdk_version = parse_version("SDK version", &field_str(&f, 2)?)?;
                check_sdk(&self.id, &self.info.sdk_version)?;
                let report: sandbox::Report = serde_json::from_slice(field(&f, 3)?)?;
                if self.sandbox.enabled && !report.gaps.is_empty() {
                    tracing::warn!(plugin = %self.id, gaps = ?report.gaps, "sandbox only partially enforced");
                }
                self.info.sandbox = self.sandbox.enabled.then(|| report.to_string());
                let pid = self.proc.as_ref().map(|p| p.child.id());
                tracing::info!(
                    plugin = %self.id, name = %self.info.name, version = %self.info.version, pid,
                    sandbox = %report, elapsed_ms = started.elapsed().as_millis() as u64, "isolated plugin loaded"
                );
                Ok(())
            }
            tag::ERR => {
                let msg = field_str(&f, 1).unwrap_or_default();
                self.kill();
                Err(PluginError::Load(msg))
            }
            other => {
                self.kill();
                Err(PluginError::Protocol(format!("unexpected reply tag {other} to LOAD")))
            }
        }
    }

    /// Send one request and wait for its reply. Any failure leaves the runner dead.
    fn request(&mut self, t: u8, fields: &[&[u8]], timeout: Duration) -> Result<Frame> {
        let Some(proc) = self.proc.as_mut() else {
            return Err(PluginError::Crashed("runner is not running".into()));
        };
        if write_frame(&mut proc.stdin, t, fields).is_err() {
            return Err(self.reap(None));
        }
        match proc.rx.recv_timeout(timeout) {
            Ok(Msg::Frame(frame)) => Ok(frame),
            Ok(Msg::Closed(reason)) => Err(self.reap(reason)),
            Err(RecvTimeoutError::Disconnected) => Err(self.reap(None)),
            Err(RecvTimeoutError::Timeout) => {
                tracing::error!(plugin = %self.id, ?timeout, "plugin call timed out; killing runner");
                self.kill();
                Err(PluginError::Timeout(timeout))
            }
        }
    }

    fn kill(&mut self) {
        if let Some(mut p) = self.proc.take() {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
    }

    /// Collect the dead runner's exit status and turn it into an error.
    fn reap(&mut self, reason: Option<String>) -> PluginError {
        let Some(mut p) = self.proc.take() else {
            return PluginError::Crashed("runner is not running".into());
        };
        // Give the OS a moment to report the exit; kill if it is merely wedged.
        let deadline = Instant::now() + Duration::from_millis(500);
        let status = loop {
            match p.child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = p.child.kill();
                    break p.child.wait().ok();
                }
            }
        };
        let mut what = status.map(describe_exit).unwrap_or_else(|| "unknown exit status".into());
        if let Some(r) = reason {
            what = format!("{what} ({r})");
        }
        tracing::error!(plugin = %self.id, status = %what, "plugin runner crashed");
        PluginError::Crashed(what)
    }

    /// Respawn a dead runner, honouring the restart budget.
    fn ensure_running(&mut self) -> Result<()> {
        if self.proc.is_some() {
            return Ok(());
        }
        let now = Instant::now();
        while self.restarts.front().is_some_and(|t| now.duration_since(*t) > self.opts.restart_window) {
            self.restarts.pop_front();
        }
        if self.restarts.len() >= self.opts.max_restarts as usize {
            return Err(PluginError::Disabled(self.id.clone()));
        }
        self.restarts.push_back(now);
        tracing::warn!(plugin = %self.id, attempt = self.restarts.len(), "restarting plugin runner");
        self.start()
    }

    /// PID of the live runner, if any.
    pub fn pid(&self) -> Option<u32> {
        self.proc.as_ref().map(|p| p.child.id())
    }
}

fn describe_exit(s: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = s.signal() {
            let name = match sig {
                libc::SIGSEGV => "SIGSEGV",
                libc::SIGABRT => "SIGABRT",
                libc::SIGBUS => "SIGBUS",
                libc::SIGILL => "SIGILL",
                libc::SIGFPE => "SIGFPE",
                libc::SIGKILL => "SIGKILL",
                _ => "signal",
            };
            return format!("killed by {name} ({sig})");
        }
    }
    match s.code() {
        Some(c) => format!("exited with code {c}"),
        None => "terminated".into(),
    }
}

impl Plugin for IsolatedPlugin {
    fn info(&self) -> &PluginInfo {
        &self.info
    }

    fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        self.ensure_running()?;
        let timeout = self.opts.call_timeout;
        let (t, f) = self.request(tag::CALL, &[method.as_bytes(), input], timeout)?;
        match t {
            tag::OK => Ok(f.into_iter().next().unwrap_or_default()),
            tag::ERR => {
                let kind = field(&f, 0)?.first().copied().unwrap_or(err_kind::OTHER);
                let message = field_str(&f, 1)?;
                Err(match kind {
                    err_kind::CALL => PluginError::Call { method: method.to_owned(), message },
                    err_kind::PANIC => PluginError::Panicked(message),
                    _ => PluginError::Load(message),
                })
            }
            other => {
                self.kill();
                Err(PluginError::Protocol(format!("unexpected reply tag {other} to CALL")))
            }
        }
    }

    fn close(mut self: Box<Self>) -> Release {
        // `stop` waits for the exit (or kills and reaps), so nothing is left.
        self.stop();
        Release::ProcessExited
    }
}

impl IsolatedPlugin {
    /// Ask the runner to shut down; kill it if it has not exited within 2 s.
    fn stop(&mut self) {
        if let Some(mut p) = self.proc.take() {
            let _ = write_frame(&mut p.stdin, tag::SHUTDOWN, &[]);
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match p.child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                    _ => {
                        let _ = p.child.kill();
                        let _ = p.child.wait();
                        break;
                    }
                }
            }
            tracing::info!(plugin = %self.id, "isolated plugin unloaded");
        }
    }
}

impl Drop for IsolatedPlugin {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(t: u8, fields: &[&[u8]]) -> Vec<u8> {
        let mut buf = Vec::new();
        write_frame(&mut buf, t, fields).unwrap();
        buf
    }

    /// Decode `bytes`, expecting an `InvalidData` error; returns its message.
    fn invalid_data(bytes: &[u8]) -> String {
        let e = read_frame(&mut Cursor::new(bytes)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{e}");
        e.to_string()
    }

    /// Writing a runner script and spawning a process never overlap across test
    /// threads, so no forked child holds a script open for writing (ETXTBSY).
    static SPAWN: Mutex<()> = Mutex::new(());

    fn spawn_lock() -> std::sync::MutexGuard<'static, ()> {
        SPAWN.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn frame_roundtrip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, tag::CALL, &[b"greet", b"", b"\x00\x01"]).unwrap();
        let (t, f) = read_frame(&mut buf.as_slice()).unwrap().unwrap();
        assert_eq!(t, tag::CALL);
        assert_eq!(f, vec![b"greet".to_vec(), vec![], vec![0, 1]]);
        assert!(read_frame(&mut &[][..]).unwrap().is_none());
    }

    #[test]
    fn every_message_kind_round_trips_through_one_stream() {
        let policy = serde_json::to_vec(&SandboxPolicy::default()).unwrap();
        let report = serde_json::to_vec(&sandbox::Report::default()).unwrap();
        let level = 3u32.to_le_bytes();
        let messages: [(u8, &[&[u8]]); 9] = [
            (tag::HELLO, &[MAGIC]),
            (tag::READY, &[MAGIC]),
            (tag::LOAD, &[b"hello", b"/plugins/libhello.so", b"{\"greeting\":\"hi\"}", policy.as_slice()]),
            (tag::LOADED, &[b"hello", b"0.1.0", b"0.1.0", report.as_slice()]),
            (tag::CALL, &[b"greet", b"\x00\xffbinary"]),
            (tag::OK, &[b""]),
            (tag::ERR, &[&[err_kind::CALL], b"bad input"]),
            (tag::LOG, &[&level, "h\u{e9}llo".as_bytes()]),
            (tag::SHUTDOWN, &[]),
        ];
        let mut stream = Vec::new();
        for (t, fields) in messages {
            write_frame(&mut stream, t, fields).unwrap();
        }
        let mut r = Cursor::new(stream);
        for (t, fields) in messages {
            let (got_t, got) = read_frame(&mut r).unwrap().expect("a frame per message");
            assert_eq!(got_t, t);
            assert_eq!(got, fields.iter().map(|f| f.to_vec()).collect::<Vec<_>>(), "tag {t}");
        }
        assert!(read_frame(&mut r).unwrap().is_none(), "clean EOF after the last frame");
    }

    #[test]
    fn field_helpers_turn_missing_and_non_utf8_fields_into_protocol_errors() {
        let f = vec![b"ok".to_vec(), vec![0xff, 0xfe]];
        assert_eq!(field(&f, 0).unwrap(), b"ok");
        assert_eq!(field_str(&f, 0).unwrap(), "ok");
        assert_eq!(field(&f, 1).unwrap(), [0xff, 0xfe], "raw fields are not UTF-8 checked");
        assert!(matches!(field(&f, 2), Err(PluginError::Protocol(m)) if m == "missing field 2"));
        assert!(matches!(field_str(&f, 1), Err(PluginError::Protocol(m)) if m == "field 1 is not UTF-8"));
        assert!(matches!(field_str(&f, 5), Err(PluginError::Protocol(m)) if m == "missing field 5"));
    }

    /// Yields `0`, then fails the test on any further read.
    struct NoMoreReads<'a>(&'a [u8]);

    impl Read for NoMoreReads<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            assert!(!self.0.is_empty(), "read past the length prefix");
            let n = buf.len().min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn declared_length_above_limit_is_rejected_before_reading_the_body() {
        for len in [MAX_FRAME as u32 + 1, u32::MAX] {
            let prefix = len.to_le_bytes();
            let e = read_frame(&mut NoMoreReads(&prefix)).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{e}");
            assert!(e.to_string().contains("out of range"), "{e}");
        }
    }

    #[test]
    fn zero_length_frame_is_rejected() {
        assert!(invalid_data(&0u32.to_le_bytes()).contains("out of range"));
    }

    #[test]
    fn rejects_oversized_and_truncated_frames() {
        let huge = ((MAX_FRAME + 1) as u32).to_le_bytes();
        assert!(read_frame(&mut &huge[..]).is_err());
        // Field claims 100 bytes but the body has 2.
        let mut bad = Vec::new();
        bad.extend_from_slice(&7u32.to_le_bytes());
        bad.push(tag::OK);
        bad.extend_from_slice(&100u32.to_le_bytes());
        bad.extend_from_slice(b"ab");
        assert!(read_frame(&mut bad.as_slice()).is_err());
    }

    #[test]
    fn frame_cut_at_any_byte_is_an_error_not_a_clean_eof() {
        let whole = frame(tag::CALL, &[b"greet", b"input"]);
        assert!(read_frame(&mut Cursor::new(&whole[..0])).unwrap().is_none());
        // Cuts inside the length prefix as well as inside the body.
        for cut in 1..whole.len() {
            let e = read_frame(&mut Cursor::new(&whole[..cut])).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof, "cut at {cut}: {e}");
        }
    }

    #[test]
    fn malformed_fields_inside_a_complete_body_are_rejected() {
        // Two stray bytes after the tag: too short for a field header.
        let mut bad = 3u32.to_le_bytes().to_vec();
        bad.extend_from_slice(&[tag::OK, 1, 2]);
        assert!(invalid_data(&bad).contains("truncated field header"));
        // A field length near u32::MAX is bounds-checked, not trusted.
        let mut bad = 5u32.to_le_bytes().to_vec();
        bad.push(tag::OK);
        bad.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(invalid_data(&bad).contains("truncated field"));
    }

    #[test]
    fn write_frame_refuses_bodies_above_the_limit_and_writes_nothing() {
        let chunk = vec![0u8; 1 << 20];
        // 64 MiB of payload plus field headers and the tag.
        let fields = vec![chunk.as_slice(); MAX_FRAME / chunk.len()];
        let mut out = Vec::new();
        let e = write_frame(&mut out, tag::OK, &fields).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e}");
        assert!(out.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn exit_status_text_names_crash_signals_and_exit_codes() {
        use std::os::unix::process::ExitStatusExt;
        // Raw wait(2) statuses: low 7 bits are the signal, the next byte the exit code.
        assert_eq!(describe_exit(ExitStatus::from_raw(libc::SIGSEGV)), "killed by SIGSEGV (11)");
        assert_eq!(describe_exit(ExitStatus::from_raw(libc::SIGSEGV | 0x80)), "killed by SIGSEGV (11)", "core bit");
        assert_eq!(describe_exit(ExitStatus::from_raw(libc::SIGKILL)), "killed by SIGKILL (9)");
        assert_eq!(
            describe_exit(ExitStatus::from_raw(libc::SIGABRT)),
            format!("killed by SIGABRT ({})", libc::SIGABRT)
        );
        assert_eq!(describe_exit(ExitStatus::from_raw(libc::SIGTERM)), format!("killed by signal ({})", libc::SIGTERM));
        assert_eq!(describe_exit(ExitStatus::from_raw(3 << 8)), "exited with code 3");
        assert_eq!(describe_exit(ExitStatus::from_raw(0)), "exited with code 0");
    }

    #[cfg(windows)]
    #[test]
    fn exit_status_text_reports_exit_codes() {
        use std::os::windows::process::ExitStatusExt;
        assert_eq!(describe_exit(ExitStatus::from_raw(3)), "exited with code 3");
    }

    /// A plugin whose runner is not running, as after a crash.
    fn crashed_plugin(runner: PathBuf, max_restarts: u32, restart_window: Duration) -> IsolatedPlugin {
        IsolatedPlugin {
            id: "p".into(),
            artifact: PathBuf::from("libp.so"),
            config: Vec::new(),
            opts: IsolationOptions {
                runner: RunnerCommand::Path(runner),
                max_restarts,
                restart_window,
                ..IsolationOptions::default()
            },
            sandbox: SandboxPolicy::disabled(),
            sink: Arc::new(|_, _| {}),
            proc: None,
            info: PluginInfo {
                name: String::new(),
                version: semver::Version::new(0, 0, 0),
                sdk_version: semver::Version::new(0, 0, 0),
                backend: Backend::Isolated,
                engine: None,
                sandbox: None,
            },
            restarts: VecDeque::new(),
        }
    }

    #[test]
    fn exhausted_restart_budget_disables_the_plugin_without_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = crashed_plugin(dir.path().join("no-runner"), 2, Duration::from_secs(60));
        let now = Instant::now();
        p.restarts.extend([now, now]);
        let e = p.call("greet", b"").unwrap_err();
        assert!(matches!(&e, PluginError::Disabled(id) if id == "p"), "{e}");
        assert_eq!(p.restarts.len(), 2, "a refused restart is not counted");

        let mut p = crashed_plugin(dir.path().join("no-runner"), 0, Duration::from_secs(60));
        assert!(matches!(p.call("greet", b""), Err(PluginError::Disabled(_))));
    }

    #[test]
    fn restarts_older_than_the_window_no_longer_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = crashed_plugin(dir.path().join("no-runner"), 1, Duration::from_millis(1));
        p.restarts.push_back(Instant::now());
        std::thread::sleep(Duration::from_millis(20));
        // The old restart expired, so a respawn is attempted; it fails because there is no runner.
        let e = {
            let _spawn = spawn_lock();
            p.call("greet", b"").unwrap_err()
        };
        assert!(matches!(&e, PluginError::Load(m) if m.contains("cannot spawn runner")), "{e}");
        assert_eq!(p.restarts.len(), 1, "only the new attempt is remembered");
    }

    #[test]
    fn request_without_a_runner_is_a_crash() {
        let mut p = crashed_plugin(PathBuf::from("no-runner"), 0, Duration::from_secs(60));
        let e = p.request(tag::CALL, &[b"greet", b""], Duration::from_secs(1)).unwrap_err();
        assert!(matches!(&e, PluginError::Crashed(m) if m == "runner is not running"), "{e}");
    }

    /// The host side against a `/bin/sh` stand-in for the runner that replays canned frames.
    #[cfg(unix)]
    mod fake_runner {
        use super::*;

        /// Print the frames stored next to the script, then exit 0. A background `cat`
        /// drains the host's requests so its writes never hit a closed pipe.
        const REPLAY: &str = "exec 3<&0\n/bin/cat <&3 >/dev/null 3<&- &\nexec /bin/cat \"$0.replies\"";

        fn spawn(script: &str, replies: &[u8], sink: LogSink) -> Result<IsolatedPlugin> {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let runner = dir.path().join("runner");
            std::fs::write(dir.path().join("runner.replies"), replies).unwrap();
            let opts = IsolationOptions {
                runner: RunnerCommand::Path(runner.clone()),
                call_timeout: Duration::from_secs(20),
                load_timeout: Duration::from_secs(20),
                max_restarts: 0,
                restart_window: Duration::from_secs(60),
            };
            let _spawn = spawn_lock();
            std::fs::write(&runner, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o755)).unwrap();
            IsolatedPlugin::spawn(
                "fake",
                Path::new("/nonexistent/libfake.so"),
                b"{}",
                opts,
                SandboxPolicy::disabled(),
                sink,
            )
        }

        fn spawn_ok(replies: &[u8], sink: LogSink) -> IsolatedPlugin {
            spawn(REPLAY, replies, sink).unwrap_or_else(|e| panic!("spawn failed: {e}"))
        }

        fn spawn_err(script: &str, replies: &[u8]) -> PluginError {
            match spawn(script, replies, Arc::new(|_, _| {})) {
                Ok(_) => panic!("spawn succeeded"),
                Err(e) => e,
            }
        }

        fn ready() -> Vec<u8> {
            frame(tag::READY, &[MAGIC])
        }

        fn loaded_with(name: &[u8], version: &[u8], sdk: &[u8]) -> Vec<u8> {
            let report = serde_json::to_vec(&sandbox::Report::default()).unwrap();
            frame(tag::LOADED, &[name, version, sdk, &report])
        }

        fn loaded() -> Vec<u8> {
            loaded_with(b"fake", b"1.2.3", plugwright_core::plugin::host_sdk_version().to_string().as_bytes())
        }

        #[test]
        fn runner_killed_by_a_signal_is_reported_as_a_crash() {
            let e = spawn_err("kill -KILL $$", &[]);
            assert!(matches!(&e, PluginError::Crashed(m) if m.contains("killed by SIGKILL (9)")), "{e}");
        }

        #[test]
        fn handshake_without_ready_and_magic_is_a_protocol_error() {
            for reply in [frame(tag::READY, &[b"NOTMAGIC"]), frame(tag::READY, &[]), frame(tag::OK, &[MAGIC])] {
                let e = spawn_err(REPLAY, &reply);
                assert!(matches!(&e, PluginError::Protocol(m) if m == "runner handshake failed"), "{e}");
            }
        }

        #[test]
        fn unknown_reply_tag_to_load_is_a_protocol_error() {
            let e = spawn_err(REPLAY, &[ready(), frame(99, &[b"?"])].concat());
            assert!(matches!(&e, PluginError::Protocol(m) if m == "unexpected reply tag 99 to LOAD"), "{e}");
        }

        #[test]
        fn load_error_reply_becomes_a_load_error() {
            let e = spawn_err(REPLAY, &[ready(), frame(tag::ERR, &[&[err_kind::LOAD], b"no such symbol"])].concat());
            assert!(matches!(&e, PluginError::Load(m) if m == "no such symbol"), "{e}");
        }

        #[test]
        fn invalid_load_metadata_is_rejected() {
            let sdk = plugwright_core::plugin::host_sdk_version().to_string();
            let e = spawn_err(REPLAY, &[ready(), loaded_with(b"\xff", b"1.0.0", sdk.as_bytes())].concat());
            assert!(matches!(&e, PluginError::Protocol(m) if m == "field 0 is not UTF-8"), "{e}");
            let e = spawn_err(REPLAY, &[ready(), loaded_with(b"fake", b"one", sdk.as_bytes())].concat());
            assert!(matches!(&e, PluginError::Load(m) if m.contains("invalid plugin version `one`")), "{e}");
            let e = spawn_err(REPLAY, &[ready(), loaded_with(b"fake", b"1.0.0", b"999.0.0")].concat());
            assert!(matches!(&e, PluginError::SdkMismatch { found, .. } if found == "999.0.0"), "{e}");
            let e = spawn_err(REPLAY, &[ready(), frame(tag::LOADED, &[b"fake", b"1.0.0", sdk.as_bytes()])].concat());
            assert!(matches!(&e, PluginError::Protocol(m) if m == "missing field 3"), "{e}");
        }

        #[test]
        fn call_replies_map_to_typed_results() {
            let logs = Arc::new(Mutex::new(Vec::new()));
            let sink: LogSink = {
                let logs = Arc::clone(&logs);
                Arc::new(move |level, msg: &str| logs.lock().unwrap().push((level, msg.len())))
            };
            let long = "x".repeat(MAX_LOG_LINE + 10);
            let replies = [
                frame(tag::LOG, &[&2u32.to_le_bytes(), b"starting"]),
                ready(),
                frame(tag::LOG, &[&4u32.to_le_bytes(), long.as_bytes()]),
                loaded(),
                frame(tag::OK, &[b"out"]),
                frame(tag::OK, &[]),
                frame(tag::ERR, &[&[err_kind::CALL], b"bad input"]),
                frame(tag::ERR, &[&[err_kind::PANIC], b"oops"]),
                frame(tag::ERR, &[&[err_kind::OTHER], b"no plugin loaded"]),
                frame(tag::ERR, &[&[200], b"from a newer runner"]),
                frame(tag::ERR, &[b"", b"no kind byte"]),
                frame(tag::ERR, &[&[err_kind::CALL], b"\xff\xfe"]),
                frame(tag::ERR, &[&[err_kind::CALL]]),
                frame(77, &[]),
            ]
            .concat();
            let mut p = spawn_ok(&replies, sink);
            assert_eq!((p.info().name.as_str(), p.info().version.to_string()), ("fake", "1.2.3".to_owned()));
            assert_eq!(p.info().sandbox, None, "sandbox disabled");
            assert_eq!(*logs.lock().unwrap(), [(2, "starting".len()), (4, MAX_LOG_LINE)], "log lines are capped");

            assert_eq!(p.call("greet", b"").unwrap(), b"out");
            assert_eq!(p.call("greet", b"").unwrap(), b"", "OK without a field is empty output");
            let e = p.call("parse", b"").unwrap_err();
            assert!(matches!(&e, PluginError::Call { method, message } if method == "parse" && message == "bad input"));
            assert!(matches!(p.call("greet", b""), Err(PluginError::Panicked(m)) if m == "oops"));
            assert!(matches!(p.call("greet", b""), Err(PluginError::Load(m)) if m == "no plugin loaded"));
            // Unknown or missing error kinds degrade to a generic error.
            assert!(matches!(p.call("greet", b""), Err(PluginError::Load(m)) if m == "from a newer runner"));
            assert!(matches!(p.call("greet", b""), Err(PluginError::Load(m)) if m == "no kind byte"));
            assert!(matches!(p.call("greet", b""), Err(PluginError::Protocol(m)) if m == "field 1 is not UTF-8"));
            assert!(matches!(p.call("greet", b""), Err(PluginError::Protocol(m)) if m == "missing field 1"));
            assert!(p.pid().is_some(), "malformed error replies keep the runner");

            let e = p.call("greet", b"").unwrap_err();
            assert!(matches!(&e, PluginError::Protocol(m) if m == "unexpected reply tag 77 to CALL"), "{e}");
            assert!(p.pid().is_none(), "an unknown reply tag kills the runner");
            assert!(matches!(p.call("greet", b""), Err(PluginError::Disabled(_))), "no restart budget");
        }

        #[test]
        fn runner_exiting_between_calls_is_a_crash_with_its_exit_status() {
            let mut p = spawn_ok(&[ready(), loaded(), frame(tag::OK, &[b"1"])].concat(), Arc::new(|_, _| {}));
            assert_eq!(p.call("greet", b"").unwrap(), b"1");
            let e = p.call("greet", b"").unwrap_err();
            assert!(matches!(&e, PluginError::Crashed(m) if m == "exited with code 0"), "{e}");
            assert!(p.pid().is_none());
            assert!(matches!(p.call("greet", b""), Err(PluginError::Disabled(_))));
        }
    }

    /// The runner loop over in-memory streams. A LOAD that got as far as the
    /// sandbox would confine this test process, so only malformed ones are sent.
    mod serve_loop {
        use super::*;

        #[derive(Clone, Default)]
        struct Sink(Arc<Mutex<Vec<u8>>>);

        impl Write for Sink {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        struct Closed;

        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        /// Feed `frames` (plus `tail` raw bytes) to the loop; exit code and replies.
        fn run(frames: &[(u8, &[&[u8]])], tail: &[u8]) -> (i32, Vec<Frame>) {
            let mut input: Vec<u8> = frames.iter().flat_map(|(t, f)| frame(*t, f)).collect();
            input.extend_from_slice(tail);
            let sink = Sink::default();
            let out: Out = Arc::new(Mutex::new(Box::new(sink.clone())));
            let code = serve(&mut Cursor::new(input), &out);
            let bytes = sink.0.lock().unwrap().clone();
            let mut r = Cursor::new(bytes);
            let mut replies = Vec::new();
            while let Some(f) = read_frame(&mut r).unwrap() {
                replies.push(f);
            }
            (code, replies)
        }

        fn err(kind: u8, msg: &str) -> Frame {
            (tag::ERR, vec![vec![kind], msg.as_bytes().to_vec()])
        }

        #[test]
        fn end_of_input_is_a_clean_exit() {
            assert_eq!(run(&[], b""), (0, vec![]));
        }

        #[test]
        fn handshake_echoes_the_magic_and_rejects_anything_else() {
            let (code, replies) = run(&[(tag::HELLO, &[MAGIC]), (tag::HELLO, &[MAGIC])], b"");
            assert_eq!(code, 0);
            assert_eq!(replies, vec![(tag::READY, vec![MAGIC.to_vec()]); 2]);
            for fields in [&[b"PLUGWRT0".as_slice()][..], &[]] {
                assert_eq!(run(&[(tag::HELLO, fields)], b""), (3, vec![]));
            }
        }

        #[test]
        fn call_before_load_is_an_error_reply_not_an_exit() {
            let (code, replies) = run(&[(tag::CALL, &[b"greet", b"x"]), (tag::CALL, &[]), (tag::HELLO, &[MAGIC])], b"");
            assert_eq!(code, 0);
            let no_plugin = err(err_kind::OTHER, "no plugin loaded");
            assert_eq!(replies, vec![no_plugin.clone(), no_plugin, (tag::READY, vec![MAGIC.to_vec()])]);
        }

        #[test]
        fn malformed_load_is_reported_before_the_sandbox_is_applied() {
            let (code, replies) = run(
                &[
                    (tag::LOAD, &[b"id"]),
                    (tag::LOAD, &[b"id", b"\xff"]),
                    (tag::LOAD, &[b"id", b"/nope", b"{}", b"not json"]),
                ],
                b"",
            );
            assert_eq!(code, 0);
            assert_eq!(replies.len(), 3);
            assert_eq!(replies[0], err(err_kind::LOAD, &PluginError::Protocol("missing field 1".into()).to_string()));
            assert_eq!(
                replies[1],
                err(err_kind::LOAD, &PluginError::Protocol("field 1 is not UTF-8".into()).to_string())
            );
            let (t, f) = &replies[2];
            assert_eq!((*t, f[0].as_slice()), (tag::ERR, [err_kind::LOAD].as_slice()));
            assert!(String::from_utf8_lossy(&f[1]).contains("expected"), "a JSON error: {:?}", f[1]);
        }

        #[test]
        fn shutdown_stops_reading() {
            let (code, replies) = run(&[(tag::SHUTDOWN, &[]), (tag::HELLO, &[MAGIC])], b"\xff");
            assert_eq!((code, replies), (0, vec![]), "nothing after SHUTDOWN is read");
        }

        #[test]
        fn protocol_errors_exit_with_3() {
            for t in [0, tag::READY, tag::OK, tag::LOG, 99] {
                assert_eq!(run(&[(t, &[])], b"").0, 3, "tag {t}");
            }
            let (code, replies) = run(&[(tag::HELLO, &[MAGIC])], &[5, 0]);
            assert_eq!((code, replies.len()), (3, 1), "a frame cut short after a valid one");
            assert_eq!(run(&[], &(MAX_FRAME as u32 + 1).to_le_bytes()).0, 3);
        }

        #[test]
        fn a_host_that_stopped_reading_ends_the_loop_with_4() {
            let out: Out = Arc::new(Mutex::new(Box::new(Closed)));
            let mut input = Cursor::new([frame(tag::HELLO, &[MAGIC]), frame(tag::HELLO, &[MAGIC])].concat());
            assert_eq!(serve(&mut input, &out), 4);
            assert!(input.position() < input.get_ref().len() as u64, "the loop stops at the first failed reply");
        }
    }
}
