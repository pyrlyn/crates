//! The binary end to end, without network: `env` and `github` only read variables and files.

use std::process::Command;

const EP: &str = "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com";
const VARS: &[&str] = &[
    "R2_ACCESS_KEY_ID",
    "R2_SECRET_ACCESS_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "SCCACHE_BUCKET",
    "SCCACHE_ENDPOINT",
    "SCCACHE_R2_READ_ONLY",
];

fn cmd() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sccache-r2"));
    for v in VARS {
        c.env_remove(v);
    }
    c
}

fn configured() -> Command {
    let mut c = cmd();
    c.env("R2_ACCESS_KEY_ID", "test-access")
        .env("R2_SECRET_ACCESS_KEY", "test-secret")
        .env("SCCACHE_BUCKET", "rust-cache")
        .env("SCCACHE_ENDPOINT", EP);
    c
}

#[test]
fn env_prints_the_backend() {
    let out = configured().args(["env", "--read-only"]).output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("RUSTC_WRAPPER=sccache\n"));
    assert!(s.contains(&format!("SCCACHE_ENDPOINT={EP}\n")));
    assert!(s.contains("SCCACHE_S3_RW_MODE=READ_ONLY\n"));
}

#[test]
fn github_writes_files_and_masks() {
    let dir = std::env::temp_dir().join(format!("sccache-r2-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (env, output) = (dir.join("env"), dir.join("output"));
    let out = configured()
        .arg("github")
        .env("GITHUB_ENV", &env)
        .env("GITHUB_OUTPUT", &output)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("::add-mask::test-secret"));
    assert!(
        std::fs::read_to_string(&env)
            .unwrap()
            .contains("AWS_SECRET_ACCESS_KEY=test-secret")
    );
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "enabled=true\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn github_without_backend_disables() {
    let dir = std::env::temp_dir().join(format!("sccache-r2-off-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (env, output) = (dir.join("env"), dir.join("output"));
    let out = cmd()
        .arg("github")
        .env("GITHUB_ENV", &env)
        .env("GITHUB_OUTPUT", &output)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(std::fs::read_to_string(&env).unwrap(), "RUSTC_WRAPPER=\n");
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "enabled=false\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_endpoint_fails() {
    let out = configured()
        .env("SCCACHE_ENDPOINT", "http://x")
        .arg("env")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

#[test]
fn unknown_command_is_usage() {
    assert_eq!(cmd().arg("nope").output().unwrap().status.code(), Some(2));
    assert_eq!(cmd().output().unwrap().status.code(), Some(2));
}
