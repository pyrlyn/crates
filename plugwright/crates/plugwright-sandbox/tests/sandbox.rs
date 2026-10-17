//! The sandbox confines the process that applies it, so each case re-runs this
//! test binary as a child with `PW_SANDBOX_CHILD` set. Needs no plugins or
//! Wasm engines, so it is cheap to run on every platform (and in containers).

use std::path::Path;
use std::process::Command;

use plugwright_sandbox::{self as sandbox, SandboxPolicy};

const CHILD: &str = "PW_SANDBOX_CHILD";

/// Runs in the child: apply the policy, try everything, print one line per probe.
fn child(dir: &Path, policy: SandboxPolicy) {
    let report = sandbox::apply(&policy, &std::env::current_exe().unwrap()).expect("apply");
    println!("report {report}");
    let ok = |name: &str, r: Result<(), String>| println!("{name} {}", if r.is_ok() { "allowed" } else { "denied" });
    ok("read", std::fs::read(dir.join("secret")).map(drop).map_err(|e| e.to_string()));
    ok("write", std::fs::write(dir.join("out"), b"x").map_err(|e| e.to_string()));
    ok("system-read", std::fs::read(system_file()).map(drop).map_err(|e| e.to_string()));
    let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string());
    ok("network", l.map(drop));
    let spawn = Command::new("/bin/sh").arg("-c").arg("true").status();
    ok("spawn", spawn.map(drop).map_err(|e| e.to_string()));
    let t = std::thread::spawn(|| 7).join().map_err(|_| "thread".to_owned());
    ok("thread", t.map(drop));
}

fn system_file() -> &'static str {
    if cfg!(target_os = "macos") { "/System/Library/CoreServices/SystemVersion.plist" } else { "/usr/lib/os-release" }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run(dir: &Path, case: &str) -> String {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_entry", "--nocapture", "--test-threads=1"])
        .env(CHILD, case)
        .env("PW_DIR", dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn has(out: &str, line: &str) -> bool {
    out.lines().any(|l| l == line)
}

#[test]
fn child_entry() {
    let Ok(case) = std::env::var(CHILD) else { return };
    let dir = std::path::PathBuf::from(std::env::var("PW_DIR").unwrap());
    let policy = match case.as_str() {
        "strict" => SandboxPolicy::default(),
        "open" => {
            SandboxPolicy { network: true, read: vec![dir.clone()], write: vec![dir.clone()], ..Default::default() }
        }
        _ => unreachable!(),
    };
    child(&dir, policy);
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn strict_policy_denies_and_open_policy_allows() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("secret"), b"s").unwrap();

    let out = run(tmp.path(), "strict");
    eprintln!("{out}");
    let landlock = !out.contains("not enforced: file");
    if landlock {
        assert!(has(&out, "read denied"), "{out}");
        assert!(has(&out, "write denied"), "{out}");
    }
    assert!(has(&out, "network denied"), "{out}");
    assert!(has(&out, "spawn denied"), "{out}");
    assert!(has(&out, "system-read allowed"), "{out}");
    assert!(has(&out, "thread allowed"), "threads must keep working: {out}");
    assert!(!tmp.path().join("out").exists());

    let out = run(tmp.path(), "open");
    eprintln!("{out}");
    for line in ["read allowed", "write allowed", "network allowed", "spawn denied", "thread allowed"] {
        assert!(has(&out, line), "{line}: {out}");
    }
}
