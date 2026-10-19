//! `lib`, `wasm` and `plugin` on the example crates: real cargo builds. The
//! cache follows `CARGO_TARGET_DIR` into `target/tmp`, so only the first run
//! pays for compilation.

use std::path::{Path, PathBuf};
use std::process::Output;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// `cargo plugwright CMD ARGS --out-dir OUT` from a directory inside the workspace.
fn pack(out: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_cargo-plugwright"))
        .arg("plugwright")
        .args(args)
        .arg("--out-dir")
        .arg(out)
        .current_dir(workspace().join("examples/host"))
        .env("CARGO_TARGET_DIR", Path::new(env!("CARGO_TARGET_TMPDIR")).join("pack-cli"))
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Runs `args` and returns the first bytes of `file` in the output directory.
fn built(out: &Path, args: &[&str], file: &str) -> Vec<u8> {
    let o = pack(out, args);
    assert!(o.status.success(), "{args:?}: {}", text(&o.stderr));
    let stdout = text(&o.stdout);
    let path = out.join(file);
    assert!(stdout.contains(&path.display().to_string()), "{args:?}: {stdout}");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{args:?}: {}: {e}", path.display()));
    bytes[..8.min(bytes.len())].to_vec()
}

fn dylib(lib: &str) -> String {
    format!("{}{lib}.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION)
}

fn is_shared(magic: &[u8]) -> bool {
    // ELF, Mach-O 64 (either byte order), PE.
    magic.starts_with(b"\x7fELF")
        || magic.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || magic.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || magic.starts_with(b"MZ")
}

#[test]
fn libraries_of_either_kind_from_any_library_crate() {
    let out = tempfile::tempdir().unwrap();
    let archive = if cfg!(target_env = "msvc") { "native_hello.lib" } else { "libnative_hello.a" };
    // A cdylib crate as a static library, and a staticlib crate as a shared one.
    assert!(built(out.path(), &["lib", "native-hello", "--static"], archive).starts_with(b"!<arch>\n"));
    assert!(is_shared(&built(out.path(), &["lib", "static-hello"], &dylib("static_hello"))));
    assert!(built(out.path(), &["wasm", "wasm-hello"], "wasm_hello.wasm").starts_with(b"\0asm"));

    let again = pack(out.path(), &["lib", "native-hello", "--static"]);
    assert!(text(&again.stdout).contains("(cached)"), "{}", text(&again.stdout));
}

#[test]
fn plugins_are_built_as_declared_and_loaded_once() {
    let out = tempfile::tempdir().unwrap();
    // A staticlib plugin comes out relinked into a shared library.
    assert!(is_shared(&built(out.path(), &["plugin", "static-hello"], &dylib("static_hello"))));
    assert!(built(out.path(), &["plugin", "wasm-hello", "--wasm"], "wasm_hello.wasm").starts_with(b"\0asm"));
    let o = pack(out.path(), &["plugin", "native-hello"]);
    assert!(text(&o.stdout).contains("; plugin "), "{}", text(&o.stdout));
}

#[test]
fn unknown_crates_name_the_workspace_packages() {
    let out = tempfile::tempdir().unwrap();
    for cmd in ["lib", "wasm", "plugin"] {
        let o = pack(out.path(), &[cmd, "no-such-crate"]);
        assert_eq!(o.status.code(), Some(1), "{cmd}: {}", text(&o.stderr));
        let err = text(&o.stderr);
        assert!(err.contains("no package `no-such-crate`") && err.contains("native-hello"), "{cmd}: {err}");
    }
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0, "nothing is written on failure");
}
