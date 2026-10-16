//! The CLI parses arguments the way cargo invokes it and reports without building.

use std::path::Path;
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cargo-plugwright"))
}

fn run(args: &[&str]) -> Output {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugwright.toml");
    // `cargo plugwright …` passes the subcommand name first.
    bin().arg("plugwright").arg("--manifest").arg(manifest).args(args).output().unwrap()
}

/// Run as `cargo plugwright ARGS` from `cwd`, with no `--manifest`.
fn run_in(cwd: &Path, args: &[&str]) -> Output {
    bin().arg("plugwright").args(args).current_dir(cwd).output().unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A project whose plugins were never built: nothing below `crates/` or `libs/` exists.
const NEVER_BUILT: &str = r#"
[[plugin]]
id = "native-one"
source = { path = "crates/native-one" }

[[plugin]]
id = "wasm-one"
kind = "wasm"
engine = "wasmi"
source = { path = "crates/wasm-one" }

[[plugin]]
id = "prebuilt"
enabled = false
source = { artifact = "libs/libprebuilt.so" }
"#;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plugwright.toml"), NEVER_BUILT).unwrap();
    dir
}

#[test]
fn list_and_doctor_report_without_building() {
    let out = run(&["list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("hello-native") && text.contains("from-git (off)"), "{text}");

    let out = run(&["doctor"]);
    let text = stdout(&out);
    for row in ["rustc", "sandbox", "accel", "relink"] {
        assert!(text.contains(row), "doctor lacks `{row}`:\n{text}");
    }
}

#[test]
fn doctor_reports_the_trust_policy_of_every_source() {
    let dir = project();
    let key = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    let manifest = format!(
        "[settings.trust]\nunsigned = \"warn\"\n{NEVER_BUILT}[plugin.trust]\nkeys = [\"{key}\"]\nunsigned = \"deny\"\n"
    );
    std::fs::write(dir.path().join("plugwright.toml"), manifest).unwrap();
    let text = stdout(&run_in(dir.path(), &["doctor"]));
    assert!(text.contains("trust        unsigned warned about, no keys"), "{text}");
    assert!(text.contains("trust        prebuilt: unsigned refused, keys RWQf6LRCGA9i…"), "{text}");
    assert_eq!(text.matches(" trust ").count(), 2, "plugins without a policy of their own are not listed:\n{text}");
}

#[test]
fn bad_usage_exits_with_2() {
    assert_eq!(run(&["frobnicate"]).status.code(), Some(2));
    assert_eq!(run(&["call", "only-id"]).status.code(), Some(2));
    let out = run(&["call", "missing", "greet"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("missing"));
}

#[test]
fn help_and_version_succeed_without_a_manifest() {
    let empty = tempfile::tempdir().unwrap();
    for flag in ["--help", "-h"] {
        let out = run_in(empty.path(), &[flag]);
        assert!(out.status.success(), "{flag}: {}", stderr(&out));
        let text = stdout(&out);
        assert!(text.starts_with("usage: cargo plugwright"), "{flag}: {text}");
        for cmd in ["build", "list", "call", "methods", "gc", "doctor", "lib", "wasm", "plugin"] {
            assert!(text.contains(&format!("\n  {cmd} ")), "{flag} lacks `{cmd}`:\n{text}");
        }
    }
    for flag in ["-V", "--version"] {
        let out = run_in(empty.path(), &[flag]);
        assert!(out.status.success(), "{flag}: {}", stderr(&out));
        assert_eq!(stdout(&out).trim(), format!("cargo-plugwright {}", env!("CARGO_PKG_VERSION")));
    }
}

#[test]
fn each_command_rejects_bad_arguments_with_usage() {
    let cases: [(&[&str], &str); 15] = [
        (&[], "no command given"),
        (&["--manifest"], "--manifest needs a path"),
        (&["frobnicate"], "unknown command `frobnicate`"),
        (&["list", "x"], "bad arguments for `list`"),
        (&["call"], "bad arguments for `call`"),
        (&["call", "id"], "bad arguments for `call`"),
        (&["call", "id", "method", "input", "extra"], "bad arguments for `call`"),
        (&["methods"], "bad arguments for `methods`"),
        (&["methods", "id", "extra"], "bad arguments for `methods`"),
        (&["gc", "extra"], "bad arguments for `gc`"),
        (&["doctor", "extra"], "bad arguments for `doctor`"),
        (&["-v", "list", "x"], "bad arguments for `list`"),
        (&["lib"], "`lib` needs a crate name"),
        (&["wasm", "x", "--static"], "unknown option `--static` for `wasm`"),
        (&["plugin", "x", "y"], "`plugin` takes one crate name"),
    ];
    let dir = project();
    let manifest = dir.path().join("plugwright.toml");
    for (args, why) in cases {
        let mut cmd = bin();
        cmd.arg("plugwright");
        // `--manifest` must stay last for the dangling-flag case.
        if args != ["--manifest"] {
            cmd.arg("--manifest").arg(&manifest);
        }
        let out = cmd.args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
        let err = stderr(&out);
        assert!(err.starts_with(&format!("error: {why}\n")), "{args:?}: {err}");
        assert!(err.contains("usage: cargo plugwright"), "{args:?}: {err}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn bad_usage_is_reported_before_looking_for_a_manifest() {
    let empty = tempfile::tempdir().unwrap();
    for (args, why) in [(&["frobnicate"][..], "unknown command"), (&["list", "x"], "bad arguments for `list`")] {
        let out = run_in(empty.path(), args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains(why), "{args:?}: {}", stderr(&out));
    }
}

#[test]
fn missing_manifest_file_is_a_clean_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.toml");
    for args in [&["list"][..], &["build"], &["gc"], &["doctor"], &["methods", "x"]] {
        let out = bin().arg("plugwright").arg("--manifest").arg(&missing).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err = stderr(&out);
        assert!(err.starts_with("error: ") && err.contains("nope.toml"), "{args:?}: {err}");
        assert!(!err.contains("panicked"), "{args:?}: {err}");
    }
}

#[test]
fn no_manifest_in_any_parent_is_a_clean_error() {
    let empty = tempfile::tempdir().unwrap();
    let out = run_in(empty.path(), &["list"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("no plugwright.toml in"), "{}", stderr(&out));
}

#[test]
fn manifest_is_found_from_a_nested_directory() {
    let dir = project();
    let nested = dir.path().join("src/deep/er");
    std::fs::create_dir_all(&nested).unwrap();
    let out = run_in(&nested, &["list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("native-one"), "{}", stdout(&out));
}

#[test]
fn list_shows_plugins_that_were_never_built() {
    let dir = project();
    let out = run_in(dir.path(), &["list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let row = |id: &str| -> Vec<String> {
        let line = text.lines().find(|l| l.starts_with(id)).unwrap_or_else(|| panic!("no `{id}` row:\n{text}"));
        line.split_whitespace().map(str::to_owned).collect()
    };
    assert_eq!(
        text.lines().next().unwrap().split_whitespace().collect::<Vec<_>>(),
        ["id", "kind", "run", "cache", "sha256", "source"]
    );
    // id, kind, run, cache, sha256, source
    let native = row("native-one");
    assert_eq!(native[1..5], ["native", "process", "-", "-"], "{text}");
    assert!(native[5].starts_with("path+") && native[5].ends_with("native-one"), "{text}");
    let wasm = row("wasm-one");
    assert_eq!(wasm[1..5], ["wasm", "wasmi", "-", "-"], "{text}");
    let prebuilt = row("prebuilt");
    assert_eq!(prebuilt[1..6], ["(off)", "native", "process", "prebuilt", "-"], "{text}");
    assert!(prebuilt[6].starts_with("artifact+"), "{text}");
}

#[test]
fn unknown_plugin_ids_fail_before_any_build() {
    let dir = project();
    for args in [&["build", "nope"][..], &["methods", "nope"], &["call", "nope", "greet"]] {
        let out = run_in(dir.path(), args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("no plugin with id `nope`"), "{args:?}: {}", stderr(&out));
    }
}

#[test]
fn gc_on_a_fresh_project_removes_nothing() {
    let dir = project();
    let out = run_in(dir.path(), &["gc"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "removed 0 cache entries");
}

#[test]
fn leading_plugwright_argument_from_cargo_is_optional() {
    let dir = project();
    let manifest = dir.path().join("plugwright.toml");
    let direct = bin().arg("--manifest").arg(&manifest).arg("list").output().unwrap();
    let via_cargo = bin().arg("plugwright").arg("--manifest").arg(&manifest).arg("list").output().unwrap();
    assert!(direct.status.success(), "{}", stderr(&direct));
    assert_eq!(direct.stdout, via_cargo.stdout);
    // Only the first argument is cargo's: a later `plugwright` is an unknown command.
    let out = bin().args(["plugwright", "plugwright"]).current_dir(dir.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}
