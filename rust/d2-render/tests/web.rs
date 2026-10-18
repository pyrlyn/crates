mod common;

use std::fs;

use d2_render::web::{data_uri, html_page, img_tag, inline_svg, strip_xml_prolog, write_html_page};
use d2_render::{Format, Renderer};

const SVG: &str = r#"<?xml version="1.0" encoding="utf-8"?><svg viewBox="0 0 1 1"></svg>"#;

#[test]
fn inline_strips_prolog() {
    assert_eq!(strip_xml_prolog(SVG), r#"<svg viewBox="0 0 1 1"></svg>"#);
    let html = inline_svg(SVG, Some("x\"y"));
    assert_eq!(
        html,
        r#"<div class="x&#34;y"><svg viewBox="0 0 1 1"></svg></div>"#
    );
}

#[test]
fn data_uri_is_base64() {
    assert_eq!(
        data_uri(b"<svg/>", Format::Svg),
        "data:image/svg+xml;base64,PHN2Zy8+"
    );
    assert!(img_tag(b"x", Format::Png, "a<b").contains("alt=\"a&lt;b\""));
}

#[test]
fn page_is_complete_html() {
    let page = html_page("A & B", SVG);
    assert!(page.starts_with("<!DOCTYPE html>"));
    assert!(page.contains("<title>A &amp; B</title>"));
    assert!(page.contains("<div class=\"d2-diagram\"><svg"));
    assert!(!page.contains("<?xml"));
}

#[cfg(feature = "cli")]
#[test]
fn page_from_real_render() {
    let Some(r) = common::real_d2("page_from_real_render") else {
        return;
    };
    page_from(r);
}

#[cfg(feature = "native")]
#[test]
fn page_from_native_render() {
    page_from(Renderer::native());
}

#[cfg(any(feature = "native", feature = "cli"))]
fn page_from(r: Renderer) {
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("a.svg");
    r.render_str("a -> b", &svg, Format::Svg).unwrap();
    let html = dir.path().join("a.html");
    write_html_page("demo", &svg, &html).unwrap();
    let text = fs::read_to_string(html).unwrap();
    assert!(text.contains("<svg"));
}
