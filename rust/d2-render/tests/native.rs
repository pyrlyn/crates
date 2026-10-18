//! The pure-Rust backend (no d2 needed), plus parity checks against the
//! CLI when feature `cli` is on and d2 is installed.

mod common;

#[cfg(feature = "cli")]
use std::collections::BTreeSet;
use std::fs;

#[cfg(feature = "cli")]
use common::real_d2;
use common::{fixture, is_svg};
use d2_render::native::{check, render_svg};
use d2_render::{Error, Format, NativeBackend, RenderOptions, Renderer, SvgInfo};

fn svg(src: &str) -> String {
    let out = render_svg(src, &RenderOptions::default()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    out.svg
}

#[test]
fn default_renderer_is_native() {
    let r = Renderer::new();
    assert_eq!(r.backend().name(), "native");
    assert!(r.version().unwrap().starts_with("d2-render-native"));
    let dir = tempfile::tempdir().unwrap();
    let report = r
        .render_str_report("x -> y", &dir.path().join("x.svg"), Format::Svg)
        .unwrap();
    assert_eq!(report.backend, "native");
}

#[test]
fn renders_fixture_without_d2() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("valid.svg");
    let report = Renderer::native()
        .render_file_report(&fixture("valid.d2"), &out, Format::Svg)
        .unwrap();
    assert_eq!(report.backend, "native");
    assert!(is_svg(&fs::read(&out).unwrap()));
    let svg = report.svg.unwrap();
    assert!(svg.width.unwrap() > 200.0);
    assert!(svg.shapes().any(|s| s == "backend.queue"));
    assert!(svg
        .connections()
        .any(|c| c == "backend.(worker -> queue)[0]"));
}

#[test]
fn showcase_renders_every_shape_kind() {
    let src = fs::read_to_string(fixture("native_showcase.d2")).unwrap();
    let out = render_svg(&src, &RenderOptions::default()).unwrap();
    let info = SvgInfo::parse(&out.svg);
    assert_eq!(info.shapes().count(), 10);
    assert_eq!(info.connections().count(), 8);
    assert!(out.svg.contains("<ellipse"));
    assert!(out.svg.contains("<polygon"));
    assert!(out.svg.contains("stroke-dasharray"));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[cfg(not(feature = "png"))]
#[test]
fn png_needs_the_png_feature() {
    use d2_render::Backend;
    assert!(!NativeBackend::new().supports(Format::Png));
    let dir = tempfile::tempdir().unwrap();
    let err = Renderer::native()
        .render_str("a", &dir.path().join("a.png"), Format::Png)
        .unwrap_err();
    assert!(matches!(
        err,
        Error::UnsupportedFormat {
            backend: "native",
            ..
        }
    ));
}

#[test]
fn diagnostics_have_positions_and_path() {
    let r = Renderer::with_backend(NativeBackend::new());
    let err = r.validate_file(&fixture("invalid.d2")).unwrap_err();
    let d = &err.diagnostics()[0];
    assert_eq!((d.line, d.column), (Some(5), Some(4)));
    assert_eq!(d.message, "maps must be terminated with }");
    assert!(d.path.as_ref().unwrap().ends_with("invalid.d2"));
    assert!(check("a -> b").is_ok());
}

#[test]
fn unsupported_features_warn_or_fail() {
    let out = render_svg(
        "t: {shape: sql_table}\nv: {shape: sequence_diagram}\nlayers: {x: {b}}\nt -> v: {label.near: top-center}\n",
        &RenderOptions::new().sketch(true).dark_theme(200),
    )
    .unwrap();
    assert_eq!(
        SvgInfo::parse(&out.svg).shapes().count(),
        2,
        "layers are not drawn"
    );
    for want in [
        "sql_table",
        "sequence_diagram",
        "--sketch",
        "--dark-theme",
        "layers",
        "label positioning",
    ] {
        assert!(
            out.warnings.iter().any(|w| w.contains(want)),
            "{want}: {:?}",
            out.warnings
        );
    }
    assert!(check("...@other").unwrap_err().is_syntax());
    assert!(check("*.style.fill: red").unwrap_err().is_syntax());
    assert!(check("vars: {a: {b: 1}}\nx: {...${a}}")
        .unwrap_err()
        .is_syntax());
    let err = check("a: ${missing}").unwrap_err();
    assert_eq!(
        err.diagnostics()[0].message,
        "could not resolve variable \"missing\""
    );
}

#[test]
fn features_fixture_renders_without_warnings() {
    let src = fs::read_to_string(fixture("native_features.d2")).unwrap();
    let out = svg(&src);
    let info = SvgInfo::parse(&out);
    let shapes: Vec<&str> = info.shapes().collect();
    for k in [
        "title",
        "legend",
        "api",
        "store",
        "grid",
        "grid.one",
        "grid.two",
        "grid.three",
    ] {
        assert!(shapes.contains(&k), "{k}: {shapes:?}");
    }
    assert_eq!(info.connections().count(), 2);
    // Theme 4 "Cool Classics" from d2-config.
    assert!(out.contains("#000536"), "theme B1 stroke");
    assert!(out.contains("Platform overview"), "var substitution");
    assert!(out.contains("<title>Primary database</title>"));
    assert!(out.contains("href=\"https://example.com\""));
    assert!(out.contains("stroke=\"red\""), "class style");
    assert!(
        out.contains("class=\"c3RvcmU= db hot\""),
        "class names like d2"
    );
}

#[test]
fn vars_and_classes() {
    let out = svg(concat!(
        "vars: {c: \"#123456\"}\n",
        "classes: {k: {style.fill: ${c}; shape: hexagon}}\n",
        "a.class: k\n",
        "b: {class: k; style.fill: \"#abcdef\"}\n",
    ));
    assert!(out.contains("fill=\"#123456\""));
    assert!(out.contains("fill=\"#abcdef\""));
    assert_eq!(out.matches("<polygon").count(), 2);
}

#[test]
fn near_constants_sit_outside_the_diagram() {
    let r = Renderer::new();
    let dir = tempfile::tempdir().unwrap();
    let report = r
        .render_str_report(
            "a -> b\nnote: Note {near: bottom-center}\n",
            &dir.path().join("n.svg"),
            Format::Svg,
        )
        .unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let src = fs::read_to_string(dir.path().join("n.svg")).unwrap();
    // The note is drawn after (below) both a and b.
    let y_of = |key: &str| -> f64 {
        let class = d2_render::svg::base64_encode(key.as_bytes());
        let at = src.find(&format!("class=\"{class}\"")).unwrap();
        let rest = &src[at..];
        let y = rest.find(" y=\"").unwrap() + 4;
        rest[y..y + rest[y..].find('"').unwrap()].parse().unwrap()
    };
    assert!(y_of("note") > y_of("b"));
}

#[test]
fn grid_icons_and_images() {
    let out = svg(concat!(
        "g: {grid-rows: 2; a; b; c; d}\n",
        "s: Server {icon: ./server.svg}\n",
        "pic: {shape: image; icon: ./logo.png}\n",
    ));
    assert_eq!(out.matches("<image ").count(), 2);
    assert!(out.contains("href=\"./server.svg\""));
}

#[test]
fn arrowheads_have_their_own_markers() {
    let out = svg(concat!(
        "a -> b: {target-arrowhead.shape: diamond}\n",
        "b <-> c: {source-arrowhead: {shape: circle; style.filled: true}; target-arrowhead: many}\n",
        "c -> d: {target-arrowhead: {shape: cf-many}}\n",
    ));
    assert_eq!(out.matches("<marker ").count(), 4);
    assert!(out.contains("marker-start="));
    assert!(out.contains(">many</tspan>"), "arrowhead label");
}

#[test]
fn themes_and_text_transform() {
    let out = render_svg(
        "a: hello {style.text-transform: uppercase}\nb: {style.font: mono}\n",
        &RenderOptions::new().theme(200),
    )
    .unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(out.svg.contains(">HELLO</tspan>"));
    assert!(out.svg.contains("monospace"));
    let terminal = render_svg("box: {a}", &RenderOptions::new().theme(300)).unwrap();
    assert!(terminal.svg.contains(">BOX</tspan>"), "CapsLock rule");
    assert!(terminal
        .warnings
        .iter()
        .any(|w| w.contains("ContainerDots")));
    let missing = render_svg("a", &RenderOptions::new().theme(9999)).unwrap();
    assert!(missing.warnings.iter().any(|w| w.contains("9999")));
}

#[test]
fn options_salt_pad_and_xml_tag() {
    let out = render_svg(
        "a -> b",
        &RenderOptions::new()
            .pad(0)
            .salt("s1")
            .no_xml_tag(true)
            .omit_version(true),
    )
    .unwrap();
    assert!(out.svg.starts_with("<svg"));
    assert!(out.svg.contains("d2n-arrow-0s1"));
    assert!(!out.svg.contains("data-d2-render-native"));
}

#[test]
fn freshness_works_with_native_backend() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("a.d2");
    fs::write(&src, "a -> b").unwrap();
    let r = Renderer::native().options(RenderOptions::new().stamp(true));
    let reports = r.refresh_dir(dir.path(), Format::Svg).unwrap();
    assert_eq!(reports.len(), 1);
    assert!(r.verify_dir(dir.path(), Format::Svg).unwrap().is_empty());
    fs::write(&src, "a -> b -> c").unwrap();
    assert_eq!(r.verify_dir(dir.path(), Format::Svg).unwrap().len(), 1);
}

/// Same shape and connection keys as the CLI for the fixtures.
#[cfg(feature = "cli")]
#[test]
fn element_keys_match_cli() {
    let Some(cli) = real_d2("element_keys_match_cli") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for name in ["valid.d2", "native_showcase.d2", "native_features.d2"] {
        let a = cli
            .render_file_report(&fixture(name), &dir.path().join("cli.svg"), Format::Svg)
            .unwrap()
            .svg
            .unwrap();
        let b = Renderer::native()
            .render_file_report(&fixture(name), &dir.path().join("native.svg"), Format::Svg)
            .unwrap()
            .svg
            .unwrap();
        let keys = |i: &SvgInfo| -> BTreeSet<String> {
            i.elements.iter().map(|e| e.key.clone()).collect()
        };
        assert_eq!(keys(&a), keys(&b), "{name}");
    }
}

/// Same diagnostics (position and message) as the CLI for the invalid fixtures.
#[cfg(feature = "cli")]
#[test]
fn diagnostics_match_cli() {
    let Some(cli) = real_d2("diagnostics_match_cli") else {
        return;
    };
    let native = Renderer::native();
    for name in ["invalid.d2", "invalid_semantic.d2", "invalid_near.d2"] {
        let strip = |e: Error| -> Vec<(Option<u32>, Option<u32>, String)> {
            e.diagnostics()
                .iter()
                .map(|d| (d.line, d.column, d.message.clone()))
                .collect()
        };
        let a = strip(cli.validate_file(&fixture(name)).unwrap_err());
        let b = strip(native.validate_file(&fixture(name)).unwrap_err());
        assert_eq!(a, b, "{name}");
    }
}
