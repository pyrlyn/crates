//! Install the agent as a per-user service that starts at login.
//!
//! launchd, systemd --user and Task Scheduler each talk only to a
//! [`ServiceManager`]. Tests drive all three against a fake on any host.
//! The identity (label, unit name, home variable) is data, so this crate is
//! not tied to one product.

mod launchd;
mod schtasks;
mod systemd;

#[cfg(test)]
mod tests;

use std::fs::{self, DirBuilder};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What the real machine would be asked to do, and nothing else.
pub trait ServiceManager: Send {
    /// `None` when the file does not exist.
    fn read(&self, path: &Path) -> io::Result<Option<String>>;
    /// Creates missing parent directories.
    fn write(&mut self, path: &Path, contents: &str) -> io::Result<()>;
    /// A file that is already gone is not an error.
    fn remove(&mut self, path: &Path) -> io::Result<()>;
    /// Creates `path` and any missing parents.
    fn create_dir_all(&mut self, path: &Path) -> io::Result<()>;
    /// An `Err` means the program could not be started. A program that ran
    /// and failed is `Ok` with [`Output::success`] set to false.
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<Output>;
}

/// stdout and stderr of one tool, without a shell in between.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// Whether the process exited with status zero.
    pub success: bool,
    /// Decoded stdout. Invalid UTF-8 is replaced.
    pub stdout: String,
    /// Decoded stderr. Invalid UTF-8 is replaced.
    pub stderr: String,
}

/// Names and paths that distinguish one product's login service from another's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceIdentity {
    /// LaunchAgent label, for example `app.cofre.agent`.
    pub launchd_label: String,
    /// systemd unit file name, for example `cofre-agent.service`.
    pub systemd_unit: String,
    /// Task Scheduler task name.
    pub task_name: String,
    /// One-line description written into the systemd unit.
    pub description: String,
    /// Environment variable that carries the app home when it is pinned.
    pub home_env: String,
    /// Flag used on Windows in place of an environment variable, for example `--home`.
    pub home_flag: String,
    /// File name under `<home>/logs` for the service manager's stdout and stderr.
    pub log_file_name: String,
}

impl ServiceIdentity {
    /// Checks that none of the names can break out of a unit file or a path.
    pub fn new(
        launchd_label: impl Into<String>,
        systemd_unit: impl Into<String>,
        task_name: impl Into<String>,
        description: impl Into<String>,
        home_env: impl Into<String>,
        home_flag: impl Into<String>,
        log_file_name: impl Into<String>,
    ) -> Result<Self, ServiceError> {
        let identity = Self {
            launchd_label: launchd_label.into(),
            systemd_unit: systemd_unit.into(),
            task_name: task_name.into(),
            description: description.into(),
            home_env: home_env.into(),
            home_flag: home_flag.into(),
            log_file_name: log_file_name.into(),
        };
        identity.check()?;
        Ok(identity)
    }

    fn check(&self) -> Result<(), ServiceError> {
        token("launchd label", &self.launchd_label, |b| {
            b.is_ascii_alphanumeric() || b == b'.' || b == b'-'
        })?;
        if !self.systemd_unit.ends_with(".service") {
            return Err(ServiceError::failed("systemd unit must end with .service"));
        }
        token("systemd unit", &self.systemd_unit, |b| {
            b.is_ascii_alphanumeric() || b == b'.' || b == b'-'
        })?;
        token("task name", &self.task_name, |b| {
            b.is_ascii_alphanumeric() || b == b'.' || b == b'-'
        })?;
        plain("description", &self.description)?;
        if !is_env_name(&self.home_env) {
            return Err(ServiceError::failed(
                "home environment variable must be an identifier",
            ));
        }
        if !self.home_flag.starts_with('-')
            || self.home_flag.contains(char::is_whitespace)
            || self.home_flag.chars().any(char::is_control)
        {
            return Err(ServiceError::failed(
                "home flag must be a single dash-prefixed word",
            ));
        }
        token("log file name", &self.log_file_name, |b| {
            b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_'
        })?;
        if self.log_file_name == "." || self.log_file_name == ".." {
            return Err(ServiceError::failed("log file name is not a file"));
        }
        Ok(())
    }
}

/// The command a login service should start, and where its log goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// Product-specific names.
    pub identity: ServiceIdentity,
    /// Absolute path of the binary, as UTF-8 with no control characters.
    pub exe: String,
    /// Arguments after the binary. Each one is a single argv entry.
    pub args: Vec<String>,
    /// App home. The service log directory is `<home>/logs`.
    pub home: String,
    /// When set, the home is passed into the service because a login service
    /// does not inherit the installing shell.
    pub home_pinned: bool,
    /// The user's home directory, where launchd and systemd keep their units.
    pub user_home: PathBuf,
}

impl Spec {
    /// Rejects paths that are not UTF-8 or that contain a control character.
    /// A newline in a unit file would start a new directive.
    pub fn new(
        identity: ServiceIdentity,
        exe: &Path,
        args: impl IntoIterator<Item = impl AsRef<str>>,
        home: &Path,
        home_pinned: bool,
        user_home: PathBuf,
    ) -> Result<Self, ServiceError> {
        let args = args
            .into_iter()
            .map(|arg| {
                let arg = arg.as_ref();
                plain("argument", arg)?;
                Ok(arg.to_owned())
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;
        Ok(Self {
            identity,
            exe: path_text(exe)?,
            args,
            home: path_text(home)?,
            home_pinned,
            user_home,
        })
    }

    /// Directory that receives the service manager's stdout and stderr.
    pub fn log_dir(&self) -> PathBuf {
        Path::new(&self.home).join("logs")
    }

    /// File the service manager appends stdout and stderr to.
    pub fn log_file(&self) -> PathBuf {
        self.log_dir().join(&self.identity.log_file_name)
    }

    /// `~/Library/LaunchAgents/<label>.plist`.
    pub fn plist_path(&self) -> PathBuf {
        self.user_home
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", self.identity.launchd_label))
    }

    /// `~/.config/systemd/user/<unit>`.
    pub fn unit_path(&self) -> PathBuf {
        self.user_home
            .join(".config/systemd/user")
            .join(&self.identity.systemd_unit)
    }
}

/// Result of an install that found an existing definition, or did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// Nothing was installed before this call.
    Fresh,
    /// A previous definition was replaced.
    Updated,
    /// The definition and the running service already matched.
    Unchanged,
}

/// Whether a login service is installed, and if it is currently running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No unit file or task is present.
    NotInstalled,
    /// The unit or task exists. `running` is the service manager's answer.
    Installed {
        /// Whether the service manager reports the process as running.
        running: bool,
    },
}

/// One operating system's login-service backend.
pub trait Backend: Send + Sync {
    /// Install and start the service.
    fn install(&self, mgr: &mut dyn ServiceManager, spec: &Spec)
    -> Result<Installed, ServiceError>;
    /// Remove the service. Removing nothing is `Ok(false)`.
    fn uninstall(&self, mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<bool, ServiceError>;
    /// Report whether the service is installed and running.
    fn status(&self, mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<Status, ServiceError>;
}

/// Backend for `os`: `"macos"`, `"linux"` or `"windows"`.
pub fn backend_for(os: &str) -> Result<Box<dyn Backend>, ServiceError> {
    match os {
        "macos" => Ok(Box::new(launchd::Launchd)),
        "linux" => Ok(Box::new(systemd::Systemd)),
        "windows" => Ok(Box::new(schtasks::Schtasks)),
        other => Err(ServiceError::failed(format!(
            "no user service backend for {other}"
        ))),
    }
}

/// Backend for this process's operating system.
pub fn current_backend() -> Result<Box<dyn Backend>, ServiceError> {
    backend_for(std::env::consts::OS)
}

/// Install with [`current_backend`].
pub fn install(mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<Installed, ServiceError> {
    current_backend()?.install(mgr, spec)
}

/// Uninstall with [`current_backend`].
pub fn uninstall(mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<bool, ServiceError> {
    current_backend()?.uninstall(mgr, spec)
}

/// Status with [`current_backend`].
pub fn status(mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<Status, ServiceError> {
    current_backend()?.status(mgr, spec)
}

/// The real filesystem and the real operating-system tools.
///
/// `run` uses [`Command`] with an argument vector, never a shell.
#[derive(Debug)]
pub struct OsServiceManager;

impl ServiceManager for OsServiceManager {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&mut self, path: &Path, contents: &str) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, contents)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
        let mut dir = DirBuilder::new();
        dir.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
        dir.create(path)
    }

    fn run(&mut self, program: &str, args: &[String]) -> io::Result<Output> {
        let out = Command::new(program).args(args).output()?;
        Ok(Output {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Why a login service could not be installed, removed or queried.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// The operation stopped because a check or a tool failed.
    #[error("{message}")]
    Failed {
        /// What went wrong, including the tool's own message when there is one.
        message: String,
    },
    /// A filesystem or spawn call failed.
    #[error("{context}: {source}")]
    Io {
        /// What was being attempted.
        context: String,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
}

impl ServiceError {
    pub(crate) fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
        }
    }

    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

pub(crate) fn query(
    mgr: &mut dyn ServiceManager,
    program: &str,
    args: &[&str],
) -> Result<Output, ServiceError> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    mgr.run(program, &args)
        .map_err(|source| ServiceError::io(format!("cannot run {program}"), source))
}

pub(crate) fn must(
    mgr: &mut dyn ServiceManager,
    program: &str,
    args: &[&str],
) -> Result<Output, ServiceError> {
    let out = query(mgr, program, args)?;
    if !out.success {
        return Err(ServiceError::failed(format!(
            "{program} {} failed: {}",
            args.join(" "),
            out.stderr.trim()
        )));
    }
    Ok(out)
}

/// Writes `contents` only when it differs, and reports whether it changed.
pub(crate) fn sync_file(
    mgr: &mut dyn ServiceManager,
    path: &Path,
    contents: &str,
) -> Result<bool, ServiceError> {
    let existing = mgr
        .read(path)
        .map_err(|source| ServiceError::io(format!("cannot read {}", path.display()), source))?;
    if existing.as_deref() == Some(contents) {
        return Ok(false);
    }
    mgr.write(path, contents)
        .map_err(|source| ServiceError::io(format!("cannot write {}", path.display()), source))?;
    Ok(true)
}

fn path_text(path: &Path) -> Result<String, ServiceError> {
    let Some(text) = path.to_str() else {
        return Err(ServiceError::failed(format!(
            "{} is not valid UTF-8",
            path.display()
        )));
    };
    plain("path", text)?;
    Ok(text.to_owned())
}

fn plain(label: &str, value: &str) -> Result<(), ServiceError> {
    if value.is_empty() {
        return Err(ServiceError::failed(format!("{label} is empty")));
    }
    if value.chars().any(char::is_control) {
        return Err(ServiceError::failed(format!(
            "{value:?} contains a control character"
        )));
    }
    Ok(())
}

fn token(label: &str, value: &str, ok: impl Fn(u8) -> bool) -> Result<(), ServiceError> {
    plain(label, value)?;
    if !value.bytes().all(ok) {
        return Err(ServiceError::failed(format!(
            "{label} contains a character that is not allowed in a unit name"
        )));
    }
    Ok(())
}

fn is_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
