//! Changed paths reach the gate's program as whole, literal arguments under the platform
//! shell: `sh` on Unix, `cmd.exe` on Windows. Gates run `git hash-object {changed}`, so each
//! path that arrives intact prints the hash of its file.

use std::fs;
use std::path::Path;
use std::process::Command as Std;

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::TempDir;

const CONFIG: &str = r#"
base = "main"

[[gate]]
name = "docs"
paths = ["docs/**"]
run = "git hash-object {changed}"
"#;

/// Spaces, cmd.exe and `sh` metacharacters, and a quote of either kind of shell; all legal in
/// Windows file names (`|<>` are not).
const NAMES: [&str; 3] = [
    "docs/a b.md",
    "docs/x & echo INJECTED & .md",
    "docs/it's (x) ^ y; z!.md",
];

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Std::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap()
}

/// A committed config and an empty `docs/`, so every file written there is a change.
fn fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "t"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    fs::write(dir.join("scoped-check.toml"), CONFIG).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "base"]);
    fs::create_dir(dir.join("docs")).unwrap();
    tmp
}

fn tool(dir: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("scoped-check");
    cmd.arg("run")
        .current_dir(dir)
        .env_remove("GITHUB_STEP_SUMMARY")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

#[test]
fn changed_paths_with_shell_metacharacters_stay_whole_arguments() {
    let tmp = fixture();
    let dir = tmp.path();
    for (i, name) in NAMES.iter().enumerate() {
        fs::write(dir.join(name), format!("file {i}\n")).unwrap();
    }

    let out = tool(dir).assert().success().get_output().stdout.clone();
    let out = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    assert!(!lines.contains(&"INJECTED"), "{out}");
    for name in NAMES {
        let hash = git(dir, &["hash-object", name]);
        assert!(
            lines.contains(&hash.trim()),
            "{name} did not arrive whole: {out}"
        );
    }
}

/// cmd.exe expands `%NAME%` even inside double quotes and has no escape for it there, so such
/// a path is a planning error (exit 3), never a gate run with an expanded variable.
#[cfg(windows)]
#[test]
fn a_percent_sign_in_a_changed_path_fails_planning_on_windows() {
    let tmp = fixture();
    let dir = tmp.path();
    fs::write(dir.join("docs/%PATH%.md"), "x\n").unwrap();
    let assert = tool(dir).assert().code(3);
    let err = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(err.contains("cmd.exe cannot quote"), "{err}");
}
