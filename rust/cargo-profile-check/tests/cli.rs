use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-profile-check"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

#[test]
fn matching_manifest_exits_zero() {
    let o = run(&fixtures(), &["match/Cargo.toml"]);
    assert_eq!(stdout(&o), "");
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn mismatch_lines_are_exact() {
    let o = run(&fixtures(), &["mismatch"]);
    assert_eq!(o.status.code(), Some(1));
    let m = "mismatch/Cargo.toml";
    let expected = [
        format!("ADD {m} profile.ci debug = false"),
        format!("ADD {m} profile.ci incremental = false"),
        format!("ADD {m} profile.ci inherits = \"dev\""),
        format!("REMOVE {m} profile.dev debug = true"),
        format!("ADD {m} profile.dev debug = \"line-tables-only\""),
        format!("REMOVE {m} profile.dev split-debuginfo = \"unpacked\""),
        format!("ADD {m} profile.dev.package.\"*\" debug = false"),
        format!("ADD {m} profile.dev.package.\"*\" opt-level = 1"),
        format!("ADD {m} profile.dist codegen-units = 1"),
        format!("ADD {m} profile.dist inherits = \"release\""),
        format!("ADD {m} profile.dist lto = \"fat\""),
        format!("REMOVE {m} profile.release codegen-units = 1"),
        format!("ADD {m} profile.release codegen-units = 16"),
    ];
    assert_eq!(stdout(&o).lines().collect::<Vec<_>>(), expected);
}

#[test]
fn line_grammar() {
    let o = run(&fixtures(), &["mismatch"]);
    for line in stdout(&o).lines() {
        let parts: Vec<_> = line.splitn(5, ' ').collect();
        assert!(matches!(parts[0], "ADD" | "REMOVE"), "{line}");
        assert!(parts[2].starts_with("profile."), "{line}");
        let (_, value) = line.split_once(" = ").unwrap();
        let parsed: toml::Table = format!("v = {value}").parse().unwrap();
        assert!(parsed.contains_key("v"));
        assert!(!line.contains('\x1b'));
    }
}

#[test]
fn inherits_chain_dist_mismatch_only_on_inherits_key() {
    let o = run(&fixtures(), &["chain"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stdout(&o),
        "REMOVE chain/Cargo.toml profile.dist inherits = \"fast\"\n\
         ADD chain/Cargo.toml profile.dist inherits = \"release\"\n"
    );
}

#[test]
fn cycle_is_error_exit_two() {
    let o = run(&fixtures(), &["cycle"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("inherits cycle"));
}

#[test]
fn workspace_walk_warns_on_member_profile() {
    let o = run(&fixtures(), &["--workspace", "members"]);
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
    let out = stdout(&o);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.starts_with("WARN members/a/Cargo.toml member crate defines [profile]"));
}

#[test]
fn json_format() {
    let o = run(&fixtures(), &["--format", "json", "mismatch"]);
    assert_eq!(o.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], false);
    let d = &v["manifests"][0]["diffs"][3];
    assert_eq!(d["op"], "REMOVE");
    assert_eq!(d["section"], "profile.dev");
    assert_eq!(d["key"], "debug");
    assert_eq!(d["value"], true);
}

#[test]
fn custom_template_and_cargo_subcommand_form() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("t.toml"),
        "[profile.release]\nlto = \"thin\"\nstrip = \"debuginfo\"\ncodegen-units = 1\n",
    )
    .unwrap();
    let t = dir.path().join("t.toml");
    let o = run(
        &fixtures(),
        &[
            "profile-check",
            "--template",
            t.to_str().unwrap(),
            "mismatch",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
}

#[test]
fn usage_error_exits_two() {
    assert_eq!(run(&fixtures(), &["--bogus"]).status.code(), Some(2));
    assert_eq!(
        run(&fixtures(), &["nope/Cargo.toml"]).status.code(),
        Some(2)
    );
}

#[test]
fn workspace_exclude_skips_dirs() {
    let o = run(
        &fixtures(),
        &[
            "--workspace",
            ".",
            "--exclude",
            "cycle",
            "--exclude",
            "mismatch",
            "--exclude",
            "chain",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
}
