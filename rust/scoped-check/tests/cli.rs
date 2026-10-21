//! End-to-end tests against a throwaway git repository holding a tiny cargo workspace
//! (`a` depends on `b`, `c` is unrelated). Gates `echo` their expanded commands, so stdout
//! shows what would run. They need a POSIX `sh`.
#![cfg(unix)]

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
paths = ["**/*.md"]
run = "echo docs {changed}"

[[gate]]
name = "test"
paths = ["**/*.rs", "**/Cargo.toml", "Cargo.lock"]
run = "echo test {packages} {nextest_filter}"

[[gate]]
name = "boom"
paths = ["**/*.txt"]
run = "echo boom; exit 3"

[[gate]]
name = "after"
paths = ["**/*.txt"]
run = "echo after"
"#;

fn git(dir: &Path, args: &[&str]) {
    let status = Std::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
}

fn member(dir: &Path, name: &str, dep: Option<&str>) {
    let deps = dep.map_or(String::new(), |d| {
        format!("{d} = {{ path = \"../{d}\" }}\n")
    });
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}"
    );
    write(dir, &format!("{name}/Cargo.toml"), &manifest);
    write(dir, &format!("{name}/src/lib.rs"), "");
}

fn fixture(config: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "t"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    write(
        dir,
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\", \"c\"]\nresolver = \"2\"\n",
    );
    member(dir, "a", Some("b"));
    member(dir, "b", None);
    member(dir, "c", None);
    write(dir, "README.md", "# fixture\n");
    write(dir, "scoped-check.toml", config);
    // The lock file is committed so later `cargo metadata` runs leave the tree clean.
    let status = Std::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["generate-lockfile", "--offline"])
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success());
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "base"]);
    tmp
}

fn tool(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = cargo_bin_cmd!("scoped-check");
    // The metadata cache sits inside `.git` (never tracked, never in the change set) so
    // each fixture has its own and tests do not touch the user's real cache.
    cmd.args(args)
        .current_dir(dir)
        .env("SCOPED_CHECK_CACHE_DIR", dir.join(".git/sc-cache"))
        .env_remove("GITHUB_STEP_SUMMARY")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("CARGO_NET_OFFLINE", "true");
    cmd
}

fn stdout_of(dir: &Path, args: &[&str]) -> String {
    let out = tool(dir, args).assert().success().get_output().clone();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn docs_only_change_runs_only_the_docs_gate() {
    let repo = fixture(CONFIG);
    write(repo.path(), "README.md", "# changed\n");
    let out = stdout_of(repo.path(), &["run"]);
    assert!(out.contains("== docs: echo docs README.md"), "{out}");
    assert!(out.contains("docs README.md\n"), "{out}");
    assert!(!out.contains("== test"), "{out}");
}

#[test]
fn change_in_b_selects_b_and_its_dependent() {
    let repo = fixture(CONFIG);
    write(repo.path(), "b/src/lib.rs", "// edit\n");
    let out = stdout_of(repo.path(), &["run"]);
    assert!(
        out.contains("test -p a -p b -E 'package(=a) | package(=b)'"),
        "{out}"
    );
    assert!(!out.contains("== docs"), "{out}");
}

#[test]
fn change_in_c_selects_only_c() {
    let repo = fixture(CONFIG);
    write(repo.path(), "c/src/lib.rs", "// edit\n");
    let out = stdout_of(repo.path(), &["plan"]);
    assert!(
        out.contains("gate test [scoped]: echo test -p c -E 'package(=c)'"),
        "{out}"
    );
}

#[test]
fn lock_file_change_selects_the_whole_workspace() {
    let repo = fixture(CONFIG);
    let lock = repo.path().join("Cargo.lock");
    fs::write(
        &lock,
        format!("{}# edit\n", fs::read_to_string(&lock).unwrap()),
    )
    .unwrap();
    let out = stdout_of(repo.path(), &["run"]);
    assert!(out.contains("== test: echo test --workspace"), "{out}");
}

#[test]
fn all_flag_ignores_the_change_set() {
    let repo = fixture(CONFIG);
    let out = stdout_of(repo.path(), &["plan", "--all"]);
    assert!(
        out.contains("gate test [--all]: echo test --workspace \n"),
        "{out}"
    );
    // With nothing changed `{changed}` has no paths, so its gate is refused, not run
    // with an empty argument list.
    assert!(
        out.contains("skip docs: {changed} has no paths: nothing changed"),
        "{out}"
    );
    assert!(!out.contains("gate docs"), "{out}");
}

#[test]
fn all_flag_substitutes_every_changed_path() {
    let repo = fixture(CONFIG);
    write(repo.path(), "README.md", "# changed\n");
    write(repo.path(), "my notes.txt", "x");
    let out = stdout_of(repo.path(), &["plan", "--all"]);
    assert!(
        out.contains("gate docs [--all]: echo docs README.md 'my notes.txt'\n"),
        "{out}"
    );
}

#[test]
fn unmatched_path_selects_every_gate() {
    let repo = fixture(CONFIG);
    write(repo.path(), "blob.bin", "x");
    let out = stdout_of(repo.path(), &["plan"]);
    assert!(
        out.contains("gate docs [all: unmatched blob.bin]: echo docs blob.bin\n"),
        "{out}"
    );
}

#[test]
fn all_flag_refuses_changed_when_the_change_set_is_unknown() {
    let repo = fixture(&with_changed_gate());
    let assert = tool(repo.path(), &["run", "--all", "--base", "nope"])
        .assert()
        .success();
    let out = text(&assert.get_output().stdout);
    assert!(
        out.contains("== fmt: skipped, {changed} has no paths: the change set is unknown"),
        "{out}"
    );
    assert!(out.contains("== test: echo test ran --workspace"), "{out}");
    assert!(!out.contains("== fmt: echo"), "{out}");
}

#[test]
fn nothing_changed_exits_zero() {
    let repo = fixture(CONFIG);
    assert_eq!(
        stdout_of(repo.path(), &["run"]),
        "nothing changed against main\n"
    );
}

#[test]
fn gate_without_affected_packages_is_skipped() {
    // A root README sits outside every package, so a cargo gate claiming it has no work.
    let config = "[[gate]]\nname = \"test\"\npaths = [\"**/*.md\"]\nrun = \"echo {packages}\"\n";
    let repo = fixture(config);
    write(repo.path(), "README.md", "# changed\n");
    let out = stdout_of(repo.path(), &["plan", "--base", "main"]);
    assert!(
        out.contains("skip test: nothing it covers changed"),
        "{out}"
    );
    assert!(!out.contains("gate test"), "{out}");
}

#[test]
fn failing_gate_stops_the_run_with_its_exit_code() {
    let repo = fixture(CONFIG);
    write(repo.path(), "notes.txt", "x");
    let assert = tool(repo.path(), &["run"]).assert().code(3);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("== boom: echo boom; exit 3"), "{out}");
    assert!(!out.contains("== after"), "{out}");
}

#[test]
fn keep_going_runs_the_rest_and_keeps_the_first_failure() {
    let repo = fixture(CONFIG);
    write(repo.path(), "notes.txt", "x");
    let assert = tool(repo.path(), &["run", "--keep-going"]).assert().code(3);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("== after: echo after"), "{out}");
}

#[test]
fn plan_json_has_the_documented_shape() {
    let repo = fixture(CONFIG);
    write(repo.path(), "b/src/lib.rs", "// edit\n");
    let json: serde_json::Value =
        serde_json::from_str(&stdout_of(repo.path(), &["plan", "--json"])).unwrap();
    assert_eq!(json["base"], "main");
    assert_eq!(json["merge_base"].as_str().unwrap().len(), 40);
    assert_eq!(json["changed"], 1);
    assert_eq!(json["nothing_changed"], false);
    assert_eq!(json["gates"][0]["name"], "test");
    assert_eq!(json["gates"][0]["why"], "scoped");
    assert!(
        json["gates"][0]["command"]
            .as_str()
            .unwrap()
            .contains("-p a -p b")
    );
    assert_eq!(json["skipped"], serde_json::json!([]));
}

#[test]
fn config_errors_exit_two() {
    for (config, message) in [
        ("[[gate]]\nname = \"x\"\npaths = [\"*\"]\n", "no `run`"),
        (
            "[[gate]]\nname = \"x\"\nrun = \"echo {nope}\"\n",
            "unknown placeholder",
        ),
        (
            "[[gate]]\nname = \"x\"\nrun = \"true\"\nbogus = [\n",
            "invalid config",
        ),
    ] {
        let repo = fixture(config);
        let assert = tool(repo.path(), &["plan"]).assert().code(2);
        let err = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
        assert!(err.contains(message), "{err}");
    }
}

#[test]
fn shell_braces_stay_literal() {
    let config =
        "[[gate]]\nname = \"x\"\npaths = [\"**/*.md\"]\nrun = \"echo ${HOME:+set} {a,b}\"\n";
    let repo = fixture(config);
    write(repo.path(), "README.md", "# changed\n");
    let out = stdout_of(repo.path(), &["plan", "--base", "main"]);
    assert!(out.contains("echo ${HOME:+set} {a,b}"), "{out}");
}

// ---- shared by the strict-mode, keep-going and cache tests ----

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// A `cargo` stand-in that logs its arguments, then runs the real cargo. `fail_metadata`
/// makes `cargo metadata` exit 101 instead. Returns (shim, log).
fn cargo_shim(dir: &Path, fail_metadata: bool) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let real = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let log = dir.join(".git/cargo.log");
    let shim = dir.join(".git/cargo-shim");
    let fail = if fail_metadata {
        "if [ \"$1\" = metadata ]; then echo 'boom from the cargo shim' >&2; exit 101; fi\n"
    } else {
        ""
    };
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{}'\n{fail}\
         if [ \"$1\" = --version ] && [ -n \"$CARGO_SHIM_VERSION\" ]; then echo \"$CARGO_SHIM_VERSION\"; exit 0; fi\n\
         exec '{real}' \"$@\"\n",
        log.display()
    );
    fs::write(&shim, script).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    (shim, log)
}

fn metadata_runs(log: &Path) -> usize {
    fs::read_to_string(log).map_or(0, |l| {
        l.lines().filter(|l| l.starts_with("metadata")).count()
    })
}

/// Three gates that need the package graph, so a scoped run asks for it three times.
const CARGO_GATES: &str = r#"
base = "main"

[[gate]]
name = "one"
paths = ["**/*.rs"]
run = "echo one {packages}"

[[gate]]
name = "two"
paths = ["**/*.rs"]
run = "echo two {packages} {nextest_filter}"

[[gate]]
name = "three"
paths = ["**/*.rs"]
run = "echo three {packages}"

[[gate]]
name = "docs"
paths = ["**/*.md"]
run = "echo docs"
"#;

// ---- planning errors are errors: exit 3, a clear message, no gate runs ----

#[test]
fn unknown_base_fails_with_exit_3_and_runs_nothing() {
    let repo = fixture(CONFIG);
    write(repo.path(), "README.md", "# changed\n");
    for args in [["plan", "--base", "nope"], ["run", "--base", "nope"]] {
        let assert = tool(repo.path(), &args).assert().code(3);
        let (out, err) = (
            text(&assert.get_output().stdout),
            text(&assert.get_output().stderr),
        );
        assert!(err.contains("error: unknown base ref: nope"), "{err}");
        assert!(
            err.contains("--fallback-all"),
            "the error names the opt-in: {err}"
        );
        assert!(!out.contains("== ") && !out.contains("gate "), "{out}");
    }
}

#[test]
fn outside_a_git_repository_fails_with_exit_3() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "scoped-check.toml", CONFIG);
    let config = tmp.path().join("scoped-check.toml");
    let assert = tool(tmp.path(), &["run", "--config", config.to_str().unwrap()])
        .env("GIT_CEILING_DIRECTORIES", tmp.path().parent().unwrap())
        .assert()
        .code(3);
    let err = text(&assert.get_output().stderr);
    assert!(err.contains("not a git repository"), "{err}");
}

#[test]
fn a_missing_git_executable_fails_with_exit_3() {
    let repo = fixture(CONFIG);
    write(repo.path(), "README.md", "# changed\n");
    let empty = TempDir::new().unwrap();
    let config = repo.path().join("scoped-check.toml");
    let assert = tool(repo.path(), &["run", "--config", config.to_str().unwrap()])
        .env("PATH", empty.path())
        .assert();
    // `sh` is not on this PATH either, but planning must fail before any gate would start.
    let assert = assert.code(3);
    let err = text(&assert.get_output().stderr);
    assert!(err.contains("git executable not found"), "{err}");
}

#[test]
fn unrelated_history_fails_with_exit_3() {
    let repo = fixture(CONFIG);
    let dir = repo.path();
    git(dir, &["checkout", "-q", "--orphan", "island"]);
    git(dir, &["rm", "-rfq", "."]);
    write(dir, "island.txt", "i\n");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "island"]);
    let config = "[[gate]]\nname = \"x\"\npaths = [\"**\"]\nrun = \"echo x\"\n";
    write(dir, "scoped-check.toml", config);
    let assert = tool(dir, &["run", "--base", "main"]).assert().code(3);
    let err = text(&assert.get_output().stderr);
    assert!(err.contains("no merge base between HEAD and main"), "{err}");
    assert!(!err.contains("shallow"), "{err}");
}

#[test]
fn shallow_clone_fails_with_exit_3_and_suggests_fetching_more() {
    let repo = fixture(CONFIG);
    let src = repo.path();
    git(src, &["checkout", "-q", "-b", "feature"]);
    write(src, "f.txt", "f\n");
    git(src, &["add", "-A"]);
    git(src, &["commit", "-q", "-m", "feature"]);
    git(src, &["checkout", "-q", "main"]);
    write(src, "m.txt", "m\n");
    git(src, &["add", "-A"]);
    git(src, &["commit", "-q", "-m", "main moves"]);

    let clone = TempDir::new().unwrap();
    let url = format!("file://{}", src.display());
    git(
        clone.path(),
        &["clone", "-q", "--depth=1", "--no-single-branch", &url, "."],
    );
    git(clone.path(), &["checkout", "-q", "feature"]);
    let assert = tool(clone.path(), &["run", "--base", "origin/main"])
        .assert()
        .code(3);
    let err = text(&assert.get_output().stderr);
    for needle in ["shallow clone", "fetch-depth: 0", "git fetch --unshallow"] {
        assert!(err.contains(needle), "missing `{needle}`: {err}");
    }
    // The advice works: after deepening, planning succeeds.
    git(clone.path(), &["fetch", "-q", "--unshallow"]);
    tool(clone.path(), &["plan", "--base", "origin/main"])
        .assert()
        .success();
}

#[test]
fn cargo_metadata_failure_fails_with_exit_3_and_runs_nothing() {
    let repo = fixture(CONFIG);
    write(repo.path(), "c/src/lib.rs", "// edit\n");
    let (shim, _log) = cargo_shim(repo.path(), true);
    for sub in ["plan", "run"] {
        let assert = tool(repo.path(), &[sub])
            .env("CARGO", &shim)
            .assert()
            .code(3);
        let (out, err) = (
            text(&assert.get_output().stdout),
            text(&assert.get_output().stderr),
        );
        assert!(err.contains("cargo metadata failed"), "{err}");
        assert!(err.contains("boom from the cargo shim"), "{err}");
        assert!(!out.contains("== "), "no gate may run: {out}");
    }
}

#[test]
fn plan_json_carries_the_error_and_still_exits_3() {
    let repo = fixture(CONFIG);
    let assert = tool(repo.path(), &["plan", "--json", "--base", "nope"])
        .assert()
        .code(3);
    let json: serde_json::Value = serde_json::from_str(&text(&assert.get_output().stdout)).unwrap();
    assert_eq!(json["error"], "unknown base ref: nope");
    assert_eq!(json["gates"], serde_json::json!([]));
    assert_eq!(json["merge_base"], serde_json::Value::Null);
}

#[test]
fn plan_json_error_is_null_on_success() {
    let repo = fixture(CONFIG);
    write(repo.path(), "README.md", "# changed\n");
    let json: serde_json::Value =
        serde_json::from_str(&stdout_of(repo.path(), &["plan", "--json"])).unwrap();
    assert_eq!(json["error"], serde_json::Value::Null);
}

#[test]
fn deliberate_all_decisions_are_not_errors() {
    // `--all` ignores the change set, even when it cannot be computed.
    let repo = fixture(CONFIG);
    let out = stdout_of(repo.path(), &["plan", "--all", "--base", "nope"]);
    assert!(out.contains("gate test [--all]"), "{out}");
    // An unmatched path and a manifest change stay plain decisions with exit 0.
    write(repo.path(), "blob.bin", "x");
    assert!(stdout_of(repo.path(), &["plan"]).contains("[all: unmatched blob.bin]"));
}

#[test]
fn config_errors_still_exit_2_not_3() {
    let repo = fixture("fallback = \"sometimes\"\n");
    let assert = tool(repo.path(), &["plan"]).assert().code(2);
    let err = text(&assert.get_output().stderr);
    assert!(err.contains("`fallback` must be"), "{err}");
}

// ---- the opt-in fallback ----

const SIMPLE: &str = r#"
base = "main"

[[gate]]
name = "docs"
paths = ["**/*.md"]
run = "echo docs ran"

[[gate]]
name = "test"
paths = ["**/*.rs"]
run = "echo test ran {packages}"
"#;

#[test]
fn fallback_flag_runs_every_gate_after_a_loud_warning() {
    let repo = fixture(SIMPLE);
    let summary = repo.path().join(".git/summary.md");
    let assert = tool(repo.path(), &["run", "--base", "nope", "--fallback-all"])
        .env("GITHUB_STEP_SUMMARY", &summary)
        .assert()
        .success();
    let (out, err) = (
        text(&assert.get_output().stdout),
        text(&assert.get_output().stderr),
    );
    assert!(
        err.contains("WARNING: planning failed (unknown base ref: nope)"),
        "{err}"
    );
    assert!(
        out.contains("== docs: echo docs ran")
            && out.contains("== test: echo test ran --workspace"),
        "{out}"
    );
    let written = fs::read_to_string(&summary).unwrap();
    assert!(
        written.contains("WARNING") && written.contains("unknown base ref: nope"),
        "{written}"
    );
}

#[test]
fn fallback_config_key_is_the_same_opt_in() {
    let repo = fixture(&format!("fallback = \"all\"\n{SIMPLE}"));
    let assert = tool(repo.path(), &["plan", "--base", "nope"])
        .assert()
        .success();
    let (out, err) = (
        text(&assert.get_output().stdout),
        text(&assert.get_output().stderr),
    );
    assert!(err.contains("WARNING"), "{err}");
    assert!(
        out.contains("gate docs [all: fallback after error: unknown base ref: nope]"),
        "{out}"
    );
    // `fallback = "error"` is the explicit default.
    let strict = fixture(&format!("fallback = \"error\"\n{SIMPLE}"));
    tool(strict.path(), &["plan", "--base", "nope"])
        .assert()
        .code(3);
}

#[test]
fn fallback_covers_a_cargo_metadata_failure_too() {
    let repo = fixture(SIMPLE);
    write(repo.path(), "c/src/lib.rs", "// edit\n");
    let (shim, _log) = cargo_shim(repo.path(), true);
    let assert = tool(repo.path(), &["run", "--fallback-all"])
        .env("CARGO", &shim)
        .assert()
        .success();
    let (out, err) = (
        text(&assert.get_output().stdout),
        text(&assert.get_output().stderr),
    );
    assert!(
        err.contains("WARNING") && err.contains("cargo metadata failed"),
        "{err}"
    );
    assert!(out.contains("== test: echo test ran --workspace"), "{out}");
}

/// [`SIMPLE`] plus a gate that substitutes `{changed}`.
fn with_changed_gate() -> String {
    format!(
        "{SIMPLE}\n[[gate]]\nname = \"fmt\"\npaths = [\"**/*.rs\"]\nrun = \"echo fmt {{changed}}\"\n"
    )
}

#[test]
fn fallback_substitutes_changed_when_the_paths_are_known() {
    // `cargo metadata` fails after the change set is known, so `{changed}` still has paths.
    let repo = fixture(&with_changed_gate());
    write(repo.path(), "c/src/lib.rs", "// edit\n");
    let (shim, _log) = cargo_shim(repo.path(), true);
    let assert = tool(repo.path(), &["run", "--fallback-all"])
        .env("CARGO", &shim)
        .assert()
        .success();
    let out = text(&assert.get_output().stdout);
    assert!(out.contains("== fmt: echo fmt c/src/lib.rs\n"), "{out}");
}

#[test]
fn fallback_refuses_changed_when_the_change_set_is_unknown() {
    let repo = fixture(&with_changed_gate());
    let args = ["plan", "--json", "--base", "nope", "--fallback-all"];
    let assert = tool(repo.path(), &args).assert().success();
    let json: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let gates: Vec<&str> = json["gates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["name"].as_str().unwrap())
        .collect();
    assert_eq!(gates, ["docs", "test"]);
    assert_eq!(json["skipped"][0]["name"], "fmt");
    assert_eq!(
        json["skipped"][0]["reason"],
        "{changed} has no paths: the change set is unknown"
    );
}

#[test]
fn fallback_never_hides_a_failing_gate() {
    let config = "[[gate]]\nname = \"bad\"\npaths = [\"**\"]\nrun = \"exit 7\"\n";
    let repo = fixture(config);
    tool(repo.path(), &["run", "--base", "nope", "--fallback-all"])
        .assert()
        .code(7);
}

#[test]
fn fallback_is_silent_when_nothing_failed() {
    let repo = fixture(SIMPLE);
    write(repo.path(), "README.md", "# changed\n");
    let assert = tool(repo.path(), &["run", "--fallback-all"])
        .assert()
        .success();
    assert!(!text(&assert.get_output().stderr).contains("WARNING"));
}

// ---- --keep-going ----

const MANY_FAILURES: &str = r#"
[[gate]]
name = "first"
paths = ["**/*.txt"]
run = "echo first; exit 4"

[[gate]]
name = "fine"
paths = ["**/*.txt"]
run = "echo fine ran"

[[gate]]
name = "second"
paths = ["**/*.txt"]
run = "echo second; exit 5"

[[gate]]
name = "last"
paths = ["**/*.txt"]
run = "echo last ran"
"#;

#[test]
fn keep_going_runs_every_gate_and_exits_with_the_first_failure() {
    let repo = fixture(&format!("base = \"main\"\n{MANY_FAILURES}"));
    write(repo.path(), "notes.txt", "x");
    let assert = tool(repo.path(), &["run", "--keep-going"]).assert().code(4);
    let (out, err) = (
        text(&assert.get_output().stdout),
        text(&assert.get_output().stderr),
    );
    for gate in ["first", "fine", "second", "last"] {
        assert!(
            out.contains(&format!("== {gate}:")),
            "{gate} did not run: {out}"
        );
    }
    assert!(err.contains("2 gate(s) failed: first, second"), "{err}");
}

#[test]
fn without_keep_going_the_first_failure_stops_the_rest() {
    let repo = fixture(&format!("base = \"main\"\n{MANY_FAILURES}"));
    write(repo.path(), "notes.txt", "x");
    let assert = tool(repo.path(), &["run"]).assert().code(4);
    let out = text(&assert.get_output().stdout);
    assert!(
        out.contains("== first:") && !out.contains("== fine:") && !out.contains("== second:"),
        "{out}"
    );
}

#[test]
fn keep_going_with_only_a_late_failure_still_fails() {
    let config = "base = \"main\"\n[[gate]]\nname = \"a\"\npaths = [\"**/*.txt\"]\nrun = \"true\"\n\
                  [[gate]]\nname = \"b\"\npaths = [\"**/*.txt\"]\nrun = \"exit 9\"\n";
    let repo = fixture(config);
    write(repo.path(), "notes.txt", "x");
    tool(repo.path(), &["run", "--keep-going"]).assert().code(9);
}

#[test]
fn keep_going_with_all_gates_passing_exits_zero() {
    let config =
        "base = \"main\"\n[[gate]]\nname = \"a\"\npaths = [\"**/*.txt\"]\nrun = \"true\"\n";
    let repo = fixture(config);
    write(repo.path(), "notes.txt", "x");
    let assert = tool(repo.path(), &["run", "--keep-going"])
        .assert()
        .success();
    assert!(!text(&assert.get_output().stderr).contains("failed"));
}

#[test]
fn a_gate_killed_by_a_signal_counts_as_exit_1() {
    let config =
        "base = \"main\"\n[[gate]]\nname = \"k\"\npaths = [\"**/*.txt\"]\nrun = \"kill -9 $$\"\n";
    let repo = fixture(config);
    write(repo.path(), "notes.txt", "x");
    tool(repo.path(), &["run", "--keep-going"]).assert().code(1);
}

// ---- speed: cargo metadata once, or not at all, and the cache ----

#[test]
fn cargo_metadata_runs_once_for_several_gates() {
    let repo = fixture(CARGO_GATES);
    write(repo.path(), "b/src/lib.rs", "// edit\n");
    let (shim, log) = cargo_shim(repo.path(), false);
    let out = text(
        &tool(repo.path(), &["run", "--no-cache"])
            .env("CARGO", &shim)
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    for gate in ["one", "two", "three"] {
        assert!(
            out.contains(&format!("== {gate}: echo {gate} -p a -p b")),
            "{out}"
        );
    }
    assert_eq!(metadata_runs(&log), 1, "three gates, one metadata run");
}

#[test]
fn cargo_metadata_is_not_run_when_the_change_cannot_need_it() {
    let repo = fixture(CARGO_GATES);
    let (shim, log) = cargo_shim(repo.path(), false);
    // A docs-only change: the only selected gate has no placeholders.
    write(repo.path(), "README.md", "# changed\n");
    let assert = tool(repo.path(), &["run", "-v"])
        .env("CARGO", &shim)
        .assert()
        .success();
    assert!(text(&assert.get_output().stderr).contains("cargo metadata: not needed"));
    // A lock-file change selects the whole workspace without asking cargo.
    let lock = repo.path().join("Cargo.lock");
    fs::write(
        &lock,
        format!("{}# edit\n", fs::read_to_string(&lock).unwrap()),
    )
    .unwrap();
    let out = text(
        &tool(repo.path(), &["plan"])
            .env("CARGO", &shim)
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert!(out.contains("echo one --workspace"), "{out}");
    // `--all` never needs the graph either.
    tool(repo.path(), &["plan", "--all"])
        .env("CARGO", &shim)
        .assert()
        .success();
    assert_eq!(metadata_runs(&log), 0);
    assert!(!log.exists(), "cargo was not run at all");
}

#[test]
fn cache_misses_then_hits_and_a_hit_skips_cargo_metadata() {
    let repo = fixture(CARGO_GATES);
    write(repo.path(), "b/src/lib.rs", "// edit\n");
    let (shim, log) = cargo_shim(repo.path(), false);
    let run = || {
        let assert = tool(repo.path(), &["plan", "-v"])
            .env("CARGO", &shim)
            .assert()
            .success();
        (
            text(&assert.get_output().stdout),
            text(&assert.get_output().stderr),
        )
    };
    let (first_out, first_err) = run();
    assert!(
        first_err.contains("cargo metadata: ran (cache miss, stored)"),
        "{first_err}"
    );
    assert_eq!(metadata_runs(&log), 1);
    let (second_out, second_err) = run();
    assert!(
        second_err.contains("cargo metadata: skipped (cache hit)"),
        "{second_err}"
    );
    assert_eq!(metadata_runs(&log), 1, "a hit must not run cargo metadata");
    assert_eq!(first_out, second_out, "the cache must not change the plan");
}

#[test]
fn cache_is_invalidated_by_a_manifest_change_a_new_cargo_version_and_no_cache() {
    let repo = fixture(CARGO_GATES);
    let (shim, log) = cargo_shim(repo.path(), false);
    let run = |extra: &[&str], version: Option<&str>| {
        let mut args = vec!["plan", "-v"];
        args.extend_from_slice(extra);
        let mut cmd = tool(repo.path(), &args);
        cmd.env("CARGO", &shim);
        if let Some(v) = version {
            cmd.env("CARGO_SHIM_VERSION", v);
        }
        text(&cmd.assert().success().get_output().stderr)
    };
    write(repo.path(), "b/src/lib.rs", "// edit\n");
    assert!(run(&[], None).contains("cache miss"));
    assert!(run(&[], None).contains("cache hit"));
    // Another cargo version: miss, then a hit under that version.
    assert!(run(&[], Some("cargo 0.0.1")).contains("cache miss"));
    assert!(run(&[], Some("cargo 0.0.1")).contains("cache hit"));
    // A manifest edited in the work tree: select a package by a source file, so the plan
    // is still scoped, and change a manifest the diff does not see as the change set by
    // committing it first.
    write(
        repo.path(),
        "c/Cargo.toml",
        &format!(
            "{}# edit\n",
            fs::read_to_string(repo.path().join("c/Cargo.toml")).unwrap()
        ),
    );
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "manifest"]);
    write(repo.path(), "b/src/lib.rs", "// edit 2\n");
    // Against HEAD the committed manifest edit is not part of the change set.
    assert!(run(&["--base", "HEAD"], None).contains("cache miss"));
    // --no-cache never reads or writes it.
    let before = metadata_runs(&log);
    assert!(run(&["--no-cache"], None).contains("cache disabled"));
    assert_eq!(metadata_runs(&log), before + 1);
}

#[test]
fn an_unwritable_cache_dir_warns_but_the_plan_still_works() {
    let repo = fixture(CARGO_GATES);
    write(repo.path(), "b/src/lib.rs", "// edit\n");
    let blocker = repo.path().join(".git/a-file");
    fs::write(&blocker, "").unwrap();
    let assert = tool(repo.path(), &["plan"])
        .env("SCOPED_CHECK_CACHE_DIR", blocker.join("cache"))
        .assert()
        .success();
    assert!(text(&assert.get_output().stderr).contains("metadata cache not stored"));
    assert!(text(&assert.get_output().stdout).contains("-p a -p b"));
}

// ---- scripts/bench.sh ----

fn bench(repo: &Path, args: &[&str]) -> std::process::Output {
    let binary = cargo_bin_cmd!("scoped-check").get_program().to_owned();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/bench.sh");
    Std::new("bash")
        .arg(script)
        .args(args)
        .current_dir(repo)
        .env("SCOPED_CHECK", binary)
        .env("SCOPED_CHECK_CACHE_DIR", repo.join(".git/sc-cache"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap()
}

#[test]
fn bench_script_times_both_runs_and_reports_the_saving() {
    let repo = fixture(SIMPLE);
    write(repo.path(), "README.md", "# changed\n");
    // `main` is the only upstream here, so the default base falls back to it.
    let out = bench(repo.path(), &[]);
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert!(out.status.success(), "{stdout}{stderr}");
    assert!(
        stdout.contains("WARNING: running the real gates, twice"),
        "{stdout}"
    );
    assert!(
        stdout.contains("== all gates: ") && stdout.contains("run --all --base"),
        "{stdout}"
    );
    // The full run executes both gates; the scoped one only the docs gate.
    assert!(
        stdout.contains("== test: echo test ran --workspace"),
        "{stdout}"
    );
    assert_eq!(
        stdout.matches("== docs: echo docs ran").count(),
        2,
        "{stdout}"
    );
    assert_eq!(stdout.matches("== test: ").count(), 1, "{stdout}");
    for label in ["all gates :", "scoped    :", "saved     :"] {
        assert!(stdout.contains(label), "missing `{label}`: {stdout}");
    }
    assert!(stdout.contains('%'), "{stdout}");
}

#[test]
fn bench_script_reports_a_failing_gate_without_aborting() {
    let config =
        "base = \"main\"\n[[gate]]\nname = \"bad\"\npaths = [\"**/*.md\"]\nrun = \"exit 5\"\n";
    let repo = fixture(config);
    write(repo.path(), "README.md", "# changed\n");
    let out = bench(repo.path(), &["main"]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert_eq!(
        stdout.matches("exit 5 after").count(),
        2,
        "both runs happened: {stdout}"
    );
    assert!(
        stdout.contains("(exit 5)") && stdout.contains("note: a gate failed"),
        "{stdout}"
    );
}

#[test]
fn bench_script_needs_to_run_in_a_repo_with_a_config() {
    let empty = TempDir::new().unwrap();
    let out = bench(empty.path(), &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("scoped-check.toml"),
        "{}",
        text(&out.stderr)
    );
}
