//! Backends against a fake that keeps files and a model of what the OS tool
//! has registered, so a wrong call order fails the way the real tool would.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use super::*;

#[derive(Debug, Default)]
struct Fake {
    files: BTreeMap<PathBuf, String>,
    dirs: BTreeSet<PathBuf>,
    log: Vec<String>,
    loaded: bool,
    running: bool,
    enabled: bool,
    task: Option<String>,
}

fn fail(stderr: &str) -> io::Result<Output> {
    Ok(Output {
        success: false,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    })
}

fn ok(stdout: &str) -> io::Result<Output> {
    Ok(Output {
        success: true,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    })
}

impl Fake {
    fn launchctl(&mut self, args: &[String]) -> io::Result<Output> {
        match (args.first().map(String::as_str), self.loaded) {
            (Some("print"), true) => ok(if self.running {
                "state = running\n"
            } else {
                "state = waiting\n"
            }),
            (Some("print"), false) => fail("not found"),
            (Some("bootstrap"), false) => {
                (self.loaded, self.running) = (true, true);
                ok("")
            }
            (Some("bootout"), true) => {
                (self.loaded, self.running) = (false, false);
                ok("")
            }
            (Some("bootstrap" | "bootout"), _) => fail("Input/output error"),
            _ => fail("unknown subcommand"),
        }
    }

    fn systemctl(&mut self, args: &[String]) -> io::Result<Output> {
        if args.first().map(String::as_str) != Some("--user") {
            return fail("missing --user");
        }
        let rest: Vec<&str> = args[1..].iter().map(String::as_str).collect();
        match rest.as_slice() {
            ["is-enabled", _] => Ok(Output {
                success: self.enabled,
                ..Output::default()
            }),
            ["is-active", _] => Ok(Output {
                success: self.running,
                ..Output::default()
            }),
            ["enable", "--now", _] => {
                (self.enabled, self.running) = (true, true);
                ok("")
            }
            ["disable", "--now", _] => {
                (self.enabled, self.running) = (false, false);
                ok("")
            }
            ["daemon-reload"] | ["restart", _] => ok(""),
            other => fail(&format!("unexpected {other:?}")),
        }
    }

    fn schtasks(&mut self, args: &[String]) -> io::Result<Output> {
        let tr = args
            .iter()
            .position(|arg| arg == "/TR")
            .and_then(|index| args.get(index + 1).cloned());
        match (args.first().map(String::as_str), self.task.is_some()) {
            (Some("/Query"), true) => ok("Status:                               Running\n"),
            (Some("/Query" | "/Run" | "/End" | "/Delete"), false) => {
                fail("ERROR: The system cannot find the file specified.")
            }
            (Some("/Create"), _) => {
                self.task = tr;
                ok("")
            }
            (Some("/Delete"), true) => {
                self.task = None;
                ok("")
            }
            (Some("/Run" | "/End"), true) => ok(""),
            _ => fail("unknown"),
        }
    }
}

impl ServiceManager for Fake {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        Ok(self.files.get(path).cloned())
    }

    fn write(&mut self, path: &Path, contents: &str) -> io::Result<()> {
        self.files.insert(path.to_owned(), contents.to_owned());
        Ok(())
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.files.remove(path);
        Ok(())
    }

    fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
        self.dirs.insert(path.to_owned());
        Ok(())
    }

    fn run(&mut self, program: &str, args: &[String]) -> io::Result<Output> {
        self.log.push(format!("{program} {}", args.join(" ")));
        match program {
            "id" => ok("501\n"),
            "launchctl" => self.launchctl(args),
            "systemctl" => self.systemctl(args),
            "schtasks" => self.schtasks(args),
            other => Err(io::Error::new(io::ErrorKind::NotFound, other.to_owned())),
        }
    }
}

fn identity() -> ServiceIdentity {
    ServiceIdentity::new(
        "app.cofre.agent",
        "cofre-agent.service",
        "cofre-agent",
        "cofre agent",
        "COFRE_HOME",
        "--home",
        "agent-service.log",
    )
    .unwrap()
}

fn spec_at(exe: &str, home_pinned: bool) -> Spec {
    Spec::new(
        identity(),
        Path::new(exe),
        ["run"],
        Path::new("/Users/me/cofre home"),
        home_pinned,
        PathBuf::from("/Users/me"),
    )
    .unwrap()
}

fn spec() -> Spec {
    spec_at("/opt/cofre/cofre", false)
}

fn changes(fake: &Fake) -> Vec<&str> {
    fake.log
        .iter()
        .map(String::as_str)
        .filter(|command| {
            !(command.starts_with("id ")
                || command.contains(" print ")
                || command.contains("is-enabled")
                || command.contains("is-active")
                || command.contains("/Query"))
        })
        .collect()
}

#[cfg(unix)]
#[test]
fn launchd_install_and_uninstall_are_idempotent() {
    let (backend, spec, mut fake) = (launchd::Launchd, spec(), Fake::default());
    let plist = PathBuf::from("/Users/me/Library/LaunchAgents/app.cofre.agent.plist");

    assert_eq!(backend.install(&mut fake, &spec).unwrap(), Installed::Fresh);
    assert!(fake.files.contains_key(&plist));
    assert!(fake.dirs.contains(Path::new("/Users/me/cofre home/logs")));
    assert_eq!(
        changes(&fake),
        ["launchctl bootstrap gui/501 /Users/me/Library/LaunchAgents/app.cofre.agent.plist"]
    );

    fake.log.clear();
    assert_eq!(
        backend.install(&mut fake, &spec).unwrap(),
        Installed::Unchanged
    );
    assert!(changes(&fake).is_empty());

    let moved = spec_at("/opt/other/cofre", false);
    assert_eq!(
        backend.install(&mut fake, &moved).unwrap(),
        Installed::Updated
    );
    assert!(fake.files[&plist].contains("/opt/other/cofre"));
    assert_eq!(
        changes(&fake),
        [
            "launchctl bootout gui/501/app.cofre.agent",
            "launchctl bootstrap gui/501 /Users/me/Library/LaunchAgents/app.cofre.agent.plist"
        ]
    );
    assert_eq!(
        backend.status(&mut fake, &spec).unwrap(),
        Status::Installed { running: true }
    );

    fake.log.clear();
    assert!(backend.uninstall(&mut fake, &spec).unwrap());
    assert!(fake.files.is_empty() && !fake.loaded);
    fake.log.clear();
    assert!(!backend.uninstall(&mut fake, &spec).unwrap());
    assert!(fake.log.is_empty());
    assert_eq!(
        backend.status(&mut fake, &spec).unwrap(),
        Status::NotInstalled
    );
}

#[cfg(unix)]
#[test]
fn launchd_reloads_a_file_that_is_present_but_not_loaded() {
    let (backend, spec, mut fake) = (launchd::Launchd, spec(), Fake::default());
    backend.install(&mut fake, &spec).unwrap();
    (fake.loaded, fake.running) = (false, false);
    fake.log.clear();
    assert_eq!(
        backend.install(&mut fake, &spec).unwrap(),
        Installed::Updated
    );
    assert_eq!(changes(&fake).len(), 1);
    assert!(fake.loaded);
}

#[cfg(unix)]
#[test]
fn systemd_install_and_uninstall_are_idempotent() {
    let (backend, spec, mut fake) = (systemd::Systemd, spec(), Fake::default());
    let unit = PathBuf::from("/Users/me/.config/systemd/user/cofre-agent.service");

    assert_eq!(backend.install(&mut fake, &spec).unwrap(), Installed::Fresh);
    assert!(fake.files.contains_key(&unit));
    assert_eq!(
        changes(&fake),
        [
            "systemctl --user daemon-reload",
            "systemctl --user enable --now cofre-agent.service"
        ]
    );

    fake.log.clear();
    assert_eq!(
        backend.install(&mut fake, &spec).unwrap(),
        Installed::Unchanged
    );
    assert!(changes(&fake).is_empty());

    let moved = spec_at("/opt/other/cofre", false);
    fake.log.clear();
    assert_eq!(
        backend.install(&mut fake, &moved).unwrap(),
        Installed::Updated
    );
    assert_eq!(
        changes(&fake),
        [
            "systemctl --user daemon-reload",
            "systemctl --user enable --now cofre-agent.service",
            "systemctl --user restart cofre-agent.service"
        ]
    );
    assert_eq!(
        backend.status(&mut fake, &spec).unwrap(),
        Status::Installed { running: true }
    );

    fake.log.clear();
    assert!(backend.uninstall(&mut fake, &spec).unwrap());
    assert_eq!(
        changes(&fake),
        [
            "systemctl --user disable --now cofre-agent.service",
            "systemctl --user daemon-reload"
        ]
    );
    assert!(fake.files.is_empty());
    fake.log.clear();
    assert!(!backend.uninstall(&mut fake, &spec).unwrap());
    assert!(fake.log.is_empty());
}

#[test]
fn schtasks_install_and_uninstall_are_idempotent() {
    let (backend, spec, mut fake) = (schtasks::Schtasks, spec(), Fake::default());

    assert_eq!(backend.install(&mut fake, &spec).unwrap(), Installed::Fresh);
    assert_eq!(fake.task.as_deref(), Some("/opt/cofre/cofre run"));
    assert_eq!(
        changes(&fake),
        [
            "schtasks /Create /SC ONLOGON /TN cofre-agent /TR /opt/cofre/cofre run /F",
            "schtasks /Run /TN cofre-agent"
        ]
    );

    fake.log.clear();
    assert_eq!(
        backend.install(&mut fake, &spec).unwrap(),
        Installed::Updated
    );
    assert_eq!(
        changes(&fake).len(),
        1,
        "a repeat install rewrites and does not start"
    );
    assert_eq!(
        backend.status(&mut fake, &spec).unwrap(),
        Status::Installed { running: true }
    );

    assert!(backend.uninstall(&mut fake, &spec).unwrap());
    assert!(fake.task.is_none());
    fake.log.clear();
    assert!(!backend.uninstall(&mut fake, &spec).unwrap());
    assert_eq!(fake.log, ["schtasks /Query /TN cofre-agent"]);
    assert_eq!(
        backend.status(&mut fake, &spec).unwrap(),
        Status::NotInstalled
    );
}

#[test]
fn a_missing_tool_is_an_error_not_a_panic() {
    let err = systemd::Systemd.install(&mut Missing, &spec()).unwrap_err();
    assert!(err.to_string().contains("cannot run systemctl"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_tool_that_fails_stops_install_with_its_message() {
    let mut fake = Fake::default();
    let spec = spec();
    launchd::Launchd.install(&mut fake, &spec).unwrap();
    fake.files.clear();
    let err = launchd::Launchd
        .install(&mut FailingBootstrap(fake), &spec)
        .unwrap_err();
    assert!(err.to_string().contains("launchctl bootstrap"), "{err}");
}

#[cfg(unix)]
struct FailingBootstrap(Fake);

#[cfg(unix)]
impl ServiceManager for FailingBootstrap {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        self.0.read(path)
    }
    fn write(&mut self, path: &Path, contents: &str) -> io::Result<()> {
        self.0.write(path, contents)
    }
    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.0.remove(path)
    }
    fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
        self.0.create_dir_all(path)
    }
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<Output> {
        if args.first().is_some_and(|arg| arg == "bootstrap") {
            return fail("Bootstrap failed: 5: Input/output error");
        }
        self.0.run(program, args)
    }
}

struct Missing;

impl ServiceManager for Missing {
    fn read(&self, _: &Path) -> io::Result<Option<String>> {
        Ok(None)
    }
    fn write(&mut self, _: &Path, _: &str) -> io::Result<()> {
        Ok(())
    }
    fn remove(&mut self, _: &Path) -> io::Result<()> {
        Ok(())
    }
    fn create_dir_all(&mut self, _: &Path) -> io::Result<()> {
        Ok(())
    }
    fn run(&mut self, program: &str, _: &[String]) -> io::Result<Output> {
        Err(io::Error::new(io::ErrorKind::NotFound, program.to_owned()))
    }
}

#[cfg(unix)]
const AWKWARD: &str = r#"/opt/my apps/"q" & <x> 'y' 50% $HOME\bin/cofre"#;

#[cfg(unix)]
#[test]
fn plist_escapes_xml_and_keeps_each_argument() {
    let plist = launchd::render(&spec_at(AWKWARD, true));
    assert!(plist.contains(
        "<string>/opt/my apps/&quot;q&quot; &amp; &lt;x&gt; &apos;y&apos; 50% $HOME\\bin/cofre</string>"
    ));
    assert!(plist.contains("<string>run</string>"));
    assert!(plist.contains("<key>COFRE_HOME</key>"));
    assert!(plist.contains("<string>/Users/me/cofre home</string>"));
    assert!(plist.contains("agent-service.log</string>"));
    assert!(plist.contains("<key>Label</key>\n  <string>app.cofre.agent</string>"));
}

#[test]
fn plist_without_a_pinned_home_has_no_environment() {
    let plist = launchd::render(&spec());
    assert!(!plist.contains("COFRE_HOME"));
    assert!(plist.starts_with("<?xml"));
}

#[cfg(unix)]
#[test]
fn unit_quotes_the_command_and_doubles_specifiers() {
    let unit = systemd::render(&spec_at(AWKWARD, true));
    assert!(
        unit.contains(r#"ExecStart="/opt/my apps/\"q\" & <x> 'y' 50%% $$HOME\\bin/cofre" "run""#)
    );
    assert!(unit.contains("Environment=\"COFRE_HOME=/Users/me/cofre home\"\n"));
    assert!(unit.contains("StandardOutput=append:/Users/me/cofre home/logs/agent-service.log\n"));
    assert!(unit.contains("Description=cofre agent\n"));
    assert!(unit.contains("WantedBy=default.target"));
}

#[test]
fn unit_environment_keeps_dollars_but_doubles_percent() {
    assert_eq!(systemd::quote("A=$x%y", false), r#""A=$x%%y""#);
    assert!(!systemd::render(&spec()).contains("Environment="));
}

#[test]
fn windows_quoting_follows_the_c_runtime_rules() {
    let q = schtasks::quote;
    assert_eq!(q(r"C:\cofre.exe"), r"C:\cofre.exe");
    assert_eq!(q(""), r#""""#);
    assert_eq!(
        q(r"C:\Program Files\cofre\cofre.exe"),
        r#""C:\Program Files\cofre\cofre.exe""#
    );
    assert_eq!(q(r#"a "b" & c"#), r#""a \"b\" & c""#);
    assert_eq!(q(r"C:\my dir\"), r#""C:\my dir\\""#);
    assert_eq!(q(r#"a\"b c"#), r#""a\\\"b c""#);
}

#[test]
fn task_command_line_carries_the_home_as_a_flag() {
    let line = schtasks::command_line(&spec_at(r"C:\Program Files\cofre\cofre.exe", true));
    assert_eq!(
        line,
        r#""C:\Program Files\cofre\cofre.exe" --home "/Users/me/cofre home" run"#
    );
    assert_eq!(
        schtasks::command_line(&spec_at(r"C:\cofre.exe", false)),
        r"C:\cofre.exe run"
    );
}

#[test]
fn paths_that_cannot_be_written_safely_are_refused() {
    let home = Path::new("/h");
    for bad in ["/a\nExecStop=/bin/evil", "/a\rb", "/a\0b", "/a\u{1b}b"] {
        let err = Spec::new(
            identity(),
            Path::new(bad),
            ["run"],
            home,
            false,
            PathBuf::new(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("control character"),
            "{bad:?} {err}"
        );
    }
    assert!(
        Spec::new(
            identity(),
            Path::new("/a"),
            ["run"],
            Path::new("/h\n"),
            true,
            PathBuf::new()
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_are_refused() {
    use std::os::unix::ffi::OsStrExt;
    let bad = Path::new(std::ffi::OsStr::from_bytes(b"/a\xffb"));
    assert!(
        Spec::new(
            identity(),
            bad,
            ["run"],
            Path::new("/h"),
            false,
            PathBuf::new()
        )
        .is_err()
    );
}

#[test]
fn names_that_could_escape_a_unit_file_are_refused() {
    assert!(
        ServiceIdentity::new(
            "../evil",
            "cofre-agent.service",
            "cofre-agent",
            "cofre agent",
            "COFRE_HOME",
            "--home",
            "agent-service.log",
        )
        .is_err()
    );
    assert!(
        ServiceIdentity::new(
            "app.cofre.agent",
            "cofre-agent.service",
            "cofre-agent",
            "cofre agent",
            "1HOME",
            "--home",
            "agent-service.log",
        )
        .is_err()
    );
    assert!(
        Spec::new(
            identity(),
            Path::new("/bin/cofre"),
            ["ok\nbad"],
            Path::new("/h"),
            false,
            PathBuf::new()
        )
        .is_err()
    );
}

#[test]
fn only_the_three_known_os_have_a_backend() {
    for os in ["macos", "linux", "windows"] {
        assert!(backend_for(os).is_ok());
    }
    assert!(backend_for("freebsd").is_err());
}

#[cfg(unix)]
#[test]
fn the_real_manager_runs_programs_without_a_shell() {
    let _spawning = crate::SPAWN_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let out = OsServiceManager
        .run("echo", &["$HOME; touch x".to_owned()])
        .unwrap();
    assert!(out.success);
    assert_eq!(out.stdout, "$HOME; touch x\n");
    assert!(OsServiceManager.run("cofre-no-such-program", &[]).is_err());
}

#[test]
fn the_real_manager_writes_creates_and_removes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a/b/unit");
    let mut sys = OsServiceManager;
    assert_eq!(sys.read(&file).unwrap(), None);
    sys.write(&file, "x").unwrap();
    assert_eq!(sys.read(&file).unwrap().as_deref(), Some("x"));
    sys.remove(&file).unwrap();
    sys.remove(&file).unwrap();
    sys.create_dir_all(&dir.path().join("logs/deep")).unwrap();
    assert!(dir.path().join("logs/deep").is_dir());
}
