//! `$D2_BIN` handling. Kept in its own test binary (one test) because it
//! mutates the process environment.

mod common;

use std::env;

use d2_render::{CliBackend, Format, Renderer, D2_BIN_ENV};

#[test]
fn d2_bin_env_overrides_path_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let real = CliBackend::new().resolve_binary().ok();

    let bogus = dir.path().join("bogus-d2");
    env::set_var(D2_BIN_ENV, &bogus);
    let err = Renderer::cli()
        .render_str("a -> b", &dir.path().join("a.svg"), Format::Svg)
        .unwrap_err();
    match &err {
        d2_render::Error::BinaryNotFound { binary, .. } => assert_eq!(binary, &bogus),
        other => panic!("expected BinaryNotFound, got {other:?}"),
    }

    // An explicit binary wins over the environment.
    if let Some(real) = &real {
        let out = dir.path().join("b.svg");
        Renderer::with_binary(real)
            .render_str("a -> b", &out, Format::Svg)
            .unwrap();
        assert!(common::is_svg(&std::fs::read(&out).unwrap()));

        env::set_var(D2_BIN_ENV, real);
        let out = dir.path().join("c.svg");
        Renderer::cli()
            .render_str("a -> b", &out, Format::Svg)
            .unwrap();
        assert!(out.exists());
    } else {
        eprintln!("SKIP d2_bin_env_overrides_path_lookup (real binary part): d2 not available");
    }

    // An empty value falls back to PATH.
    env::set_var(D2_BIN_ENV, "");
    assert_eq!(CliBackend::new().resolve_binary().ok(), real);
    env::remove_var(D2_BIN_ENV);
}
