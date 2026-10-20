//! Tests against the real `d2` CLI; each soft-skips when d2 is missing.

mod common;

use std::fs;
use std::path::Path;

use common::{fixture, is_svg, real_d2};
use d2_render::{Error, Format, Freshness, Layout, RenderOptions, Renderer};

#[test]
fn renders_valid_svg() {
    let Some(r) = real_d2("renders_valid_svg") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("nested/valid.svg");
    let report = r
        .render_file_report(&fixture("valid.d2"), &out, Format::Svg)
        .unwrap();
    assert_eq!(report.output, out);
    assert_eq!(report.backend, "cli");
    assert!(is_svg(&fs::read(&out).unwrap()));
    let svg = report.svg.as_ref().unwrap();
    assert!(svg.width.unwrap() > 0.0 && svg.height.unwrap() > 0.0);
    assert!(svg.d2_version.as_deref().unwrap_or("").starts_with('v'));
    let shapes: Vec<_> = svg.shapes().collect();
    for k in [
        "user",
        "web",
        "api",
        "db",
        "backend",
        "backend.worker",
        "backend.queue",
    ] {
        assert!(shapes.contains(&k), "missing shape {k} in {shapes:?}");
    }
    let conns: Vec<_> = svg.connections().collect();
    assert!(conns.contains(&"(api -> backend.queue)[0]"), "{conns:?}");
    assert!(conns.contains(&"backend.(worker -> queue)[0]"), "{conns:?}");
    assert!(report.to_string().contains("valid.svg"));
}

#[test]
fn render_str_writes_svg() {
    let Some(r) = real_d2("render_str_writes_svg") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("ab.svg");
    let p = r.render_str("a -> b: hi", &out, Format::Svg).unwrap();
    assert_eq!(p, out);
    assert!(is_svg(&fs::read(&out).unwrap()));
}

#[test]
fn renders_png_when_supported() {
    let Some(r) = real_d2("renders_png_when_supported") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("valid.png");
    match r.render_file(&fixture("valid.d2"), &out, Format::Png) {
        Ok(p) => {
            let bytes = fs::read(p).unwrap();
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        }
        Err(e) => {
            let msg = format!("{e} {e:?}").to_lowercase();
            if msg.contains("playwright") || msg.contains("chromium") || msg.contains("browser") {
                eprintln!("SKIP renders_png_when_supported: PNG export unavailable: {e}");
            } else {
                panic!("PNG render failed: {e:?}");
            }
        }
    }
}

#[test]
fn format_follows_enum_not_extension() {
    let Some(r) = real_d2("format_follows_enum_not_extension") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("diagram.out");
    r.render_str("a -> b", &out, Format::Svg).unwrap();
    assert!(is_svg(&fs::read(&out).unwrap()));
    // No temporary files left behind.
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn renders_txt() {
    let Some(r) = real_d2("renders_txt") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("ab.txt");
    let report = r.render_auto(&fixture("valid.d2"), &out).unwrap();
    assert_eq!(report.format, Format::Txt);
    assert!(fs::read_to_string(out).unwrap().contains("Database"));
}

#[test]
fn invalid_source_is_syntax_error_with_position() {
    let Some(r) = real_d2("invalid_source_is_syntax_error_with_position") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let err = r
        .render_file(
            &fixture("invalid.d2"),
            &dir.path().join("x.svg"),
            Format::Svg,
        )
        .unwrap_err();
    let Error::Syntax {
        diagnostics,
        stderr,
    } = &err
    else {
        panic!("expected Syntax, got {err:?}");
    };
    assert!(stderr.contains("maps must be terminated"));
    assert_eq!(diagnostics.len(), 1);
    let d = &diagnostics[0];
    assert_eq!((d.line, d.column), (Some(5), Some(4)));
    assert_eq!(d.message, "maps must be terminated with }");
    assert!(d.path.as_ref().unwrap().ends_with("invalid.d2"));
    assert!(!dir.path().join("x.svg").exists());
}

#[test]
fn semantic_errors_are_all_reported() {
    let Some(r) = real_d2("semantic_errors_are_all_reported") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let err = r
        .render_file(
            &fixture("invalid_semantic.d2"),
            &dir.path().join("x.svg"),
            Format::Svg,
        )
        .unwrap_err();
    let pos: Vec<_> = err
        .diagnostics()
        .iter()
        .map(|d| (d.line.unwrap(), d.column.unwrap(), d.message.as_str()))
        .collect();
    assert_eq!(
        pos,
        [
            (1, 12, "unknown shape \"nosuch\""),
            (
                2,
                18,
                "expected \"opacity\" to be a number between 0.0 and 1.0"
            ),
        ]
    );
}

#[test]
fn validate_uses_d2_validate() {
    let Some(r) = real_d2("validate_uses_d2_validate") else {
        return;
    };
    r.validate_file(&fixture("valid.d2")).unwrap();
    r.validate_str("a -> b").unwrap();
    let err = r.validate_file(&fixture("invalid.d2")).unwrap_err();
    assert!(err.is_syntax(), "{err:?}");
    let d = &err.diagnostics()[0];
    assert_eq!((d.line, d.column), (Some(5), Some(4)));
    let err = r.validate_str("a -> \n").unwrap_err();
    assert_eq!(
        err.diagnostics()[0].message,
        "connection missing destination"
    );
    // `d2 validate` alone accepts semantic errors; the full check does not.
    let cli = d2_render::CliBackend::new();
    cli.validate_syntax(&fixture("invalid_semantic.d2"))
        .unwrap();
    let err = r
        .validate_file(&fixture("invalid_semantic.d2"))
        .unwrap_err();
    assert_eq!(err.diagnostics().len(), 2, "{err:?}");
}

#[test]
fn options_are_passed_through() {
    let Some(r) = real_d2("options_are_passed_through") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let r = r.options(
        RenderOptions::new()
            .theme(200)
            .layout(Layout::Elk)
            .pad(5)
            .sketch(true)
            .omit_version(true)
            .no_xml_tag(true),
    );
    let out = dir.path().join("opts.svg");
    let report = r
        .render_file_report(&fixture("valid.d2"), &out, Format::Svg)
        .unwrap();
    let text = fs::read_to_string(&out).unwrap();
    assert!(text.starts_with("<svg"), "--no-xml-tag not applied");
    assert!(
        report.svg.unwrap().d2_version.is_none(),
        "--omit-version not applied"
    );
}

#[test]
fn bad_theme_is_failed_not_syntax() {
    let Some(r) = real_d2("bad_theme_is_failed_not_syntax") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let err = r
        .options(RenderOptions::new().theme(9999))
        .render_str("a", &dir.path().join("a.svg"), Format::Svg)
        .unwrap_err();
    match err {
        Error::Failed { code, messages, .. } => {
            assert_eq!(code, Some(1));
            assert!(
                messages.iter().any(|m| m.contains("could not be found")),
                "{messages:?}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn freshness_roundtrip() {
    let Some(r) = real_d2("freshness_roundtrip") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("a.d2");
    let out = dir.path().join("a.svg");
    fs::write(&src, "a -> b").unwrap();
    assert_eq!(r.freshness(&src, &out).unwrap(), Freshness::Missing);
    r.render_file(&src, &out, Format::Svg).unwrap();
    assert_eq!(r.freshness(&src, &out).unwrap(), Freshness::Unstamped);

    let stamped = r.clone().options(RenderOptions::new().stamp(true));
    let report = stamped.render_file_report(&src, &out, Format::Svg).unwrap();
    assert_eq!(
        report.svg.unwrap().source_hash.as_deref(),
        Some(report.source_hash.as_str())
    );
    assert!(!stamped.is_stale(&src, &out).unwrap());
    // Different options make it stale too.
    let themed = RenderOptions::new().stamp(true).theme(1);
    assert!(r.clone().options(themed).is_stale(&src, &out).unwrap());

    fs::write(&src, "a -> c").unwrap();
    assert!(matches!(
        stamped.freshness(&src, &out).unwrap(),
        Freshness::Outdated { .. }
    ));
    let stale = stamped.verify_dir(dir.path(), Format::Svg).unwrap();
    assert_eq!(stale.len(), 1);
    let refreshed = stamped.refresh_dir(dir.path(), Format::Svg).unwrap();
    assert_eq!(refreshed.len(), 1);
    assert!(stamped
        .verify_dir(dir.path(), Format::Svg)
        .unwrap()
        .is_empty());

    // PNG uses a sidecar.
    let png = dir.path().join("a.png");
    if stamped.render_file(&src, &png, Format::Png).is_ok() {
        assert!(dir.path().join("a.png.d2hash").exists());
        assert!(!stamped.is_stale(&src, &png).unwrap());
    }
}

#[test]
fn fmt_check_and_format() {
    let Some(_) = real_d2("fmt_check_and_format") else {
        return;
    };
    let cli = d2_render::CliBackend::new();
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("u.d2");
    fs::copy(fixture("unformatted.d2"), &p).unwrap();
    assert!(!cli.is_formatted(&p).unwrap());
    cli.format_file(&p).unwrap();
    assert!(cli.is_formatted(&p).unwrap());
    assert_eq!(fs::read_to_string(&p).unwrap().trim(), "a -> b");
}

#[test]
fn version_is_reported() {
    let Some(r) = real_d2("version_is_reported") else {
        return;
    };
    assert!(r.version().unwrap().starts_with('v'));
}

#[test]
fn bogus_binary_is_binary_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("no/such/d2");
    let r = Renderer::with_binary(&bogus);
    let err = r
        .render_str("a -> b", &dir.path().join("a.svg"), Format::Svg)
        .unwrap_err();
    match &err {
        Error::BinaryNotFound { binary, .. } => assert_eq!(binary, &bogus),
        other => panic!("expected BinaryNotFound, got {other:?}"),
    }
    assert!(err.is_binary_not_found());
    assert!(err.to_string().contains("D2_BIN"));
    assert!(r.validate_str("a").unwrap_err().is_binary_not_found());
    assert!(r.version().unwrap_err().is_binary_not_found());
}

#[test]
fn bare_name_not_on_path_is_binary_not_found() {
    let r = Renderer::with_binary("d2-render-definitely-not-installed");
    assert!(r.version().unwrap_err().is_binary_not_found());
}

#[test]
fn unknown_extension_for_render_auto() {
    let r = Renderer::with_binary("unused");
    let err = r
        .render_auto(Path::new("x.d2"), Path::new("x.unknown"))
        .unwrap_err();
    assert!(matches!(err, Error::UnknownFormat(_)));
}
