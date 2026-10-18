//! The CLI backend against scripted stand-ins for d2, so error mapping is
//! tested without a real d2 install.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use d2_render::{Error, Format, RenderOptions, Renderer};

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn compile_errors_become_syntax() {
    let dir = tempfile::tempdir().unwrap();
    let bin = script(
        dir.path(),
        "d2",
        r#"echo "err: failed to compile in.d2: /x/in.d2:3:7: unknown shape \"blob\"" >&2
echo "err: /x/in.d2:4:1: connection missing destination" >&2
exit 1"#,
    );
    let err = Renderer::with_binary(bin)
        .render_str("x", &dir.path().join("o.svg"), Format::Svg)
        .unwrap_err();
    let ds = err.diagnostics();
    assert_eq!(ds.len(), 2);
    assert_eq!((ds[0].line, ds[0].column), (Some(3), Some(7)));
    assert_eq!(ds[1].message, "connection missing destination");
    assert!(err.to_string().contains("unknown shape"));
}

#[test]
fn other_failures_become_failed() {
    let dir = tempfile::tempdir().unwrap();
    let bin = script(
        dir.path(),
        "d2",
        r#"echo "err: bad usage: -t[heme] could not be found." >&2
exit 3"#,
    );
    match Renderer::with_binary(bin)
        .render_str("x", &dir.path().join("o.svg"), Format::Svg)
        .unwrap_err()
    {
        Error::Failed {
            code,
            messages,
            partial,
            ..
        } => {
            assert_eq!(code, Some(3));
            assert!(!partial);
            assert_eq!(messages, ["bad usage: -t[heme] could not be found."]);
        }
        e => panic!("{e:?}"),
    }
}

#[test]
fn partial_render_is_failed_with_flag() {
    let dir = tempfile::tempdir().unwrap();
    let bin = script(
        dir.path(),
        "d2",
        r#"echo "err: failed to fully compile (partial render written) in.d2: failed to bundle remote images" >&2
exit 1"#,
    );
    let err = Renderer::with_binary(bin)
        .render_str("x", &dir.path().join("o.svg"), Format::Svg)
        .unwrap_err();
    assert!(
        matches!(err, Error::Failed { partial: true, .. }),
        "{err:?}"
    );
}

#[test]
fn args_warnings_and_report() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.txt");
    // Record the arguments, write a tiny SVG to the last one, warn once.
    let bin = script(
        dir.path(),
        "d2",
        &format!(
            r#"echo "$@" > '{log}'
for last; do :; done
printf '<svg viewBox="0 0 10 20"><g class="YQ=="></g></svg>' > "$last"
echo "warn: something minor" >&2
echo "success: successfully compiled in.d2 to out.svg in 1.5ms" >&2"#,
            log = log.display()
        ),
    );
    let r = Renderer::with_binary(bin).options(RenderOptions::new().theme(3).pad(7).sketch(true));
    let out = dir.path().join("o.svg");
    let report = r.render_str_report("a", &out, Format::Svg).unwrap();
    let args = fs::read_to_string(&log).unwrap();
    assert!(
        args.starts_with("--theme=3 --pad=7 --sketch --watch=false "),
        "{args}"
    );
    assert!(args.trim_end().ends_with("o.svg"));
    assert_eq!(report.warnings, ["something minor"]);
    let svg = report.svg.unwrap();
    assert_eq!(svg.width, Some(10.0));
    assert_eq!(svg.shapes().collect::<Vec<_>>(), ["a"]);
}

#[test]
fn non_executable_file_is_binary_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("d2");
    fs::write(&p, "not a program").unwrap();
    let err = Renderer::with_binary(&p).version().unwrap_err();
    assert!(err.is_binary_not_found(), "{err:?}");
}
