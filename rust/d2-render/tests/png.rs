//! PNG from the native backend through resvg (feature `png`); no d2 needed.

use std::fs;

use d2_render::{Backend, Format, NativeBackend, RenderOptions, Renderer};

fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let be = |r: std::ops::Range<usize>| u32::from_be_bytes(bytes[r].try_into().unwrap());
    (be(16..20), be(20..24))
}

#[test]
fn renders_png_natively() {
    assert!(NativeBackend::new().supports(Format::Png));
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("a.png");
    let report = Renderer::new()
        .render_str_report("a -> b: hi", &out, Format::Png)
        .unwrap();
    assert_eq!(report.backend, "native");
    assert!(report.svg.is_none());
    let (w, h) = png_size(&fs::read(&out).unwrap());
    assert!(w > 200 && h > 400, "{w}x{h}");
}

#[test]
fn scale_sets_pixel_ratio() {
    let dir = tempfile::tempdir().unwrap();
    let r1 = Renderer::new().options(RenderOptions::new().scale(1.0).pad(0));
    let r2 = Renderer::new().options(RenderOptions::new().scale(3.0).pad(0));
    r1.render_str("a", &dir.path().join("1.png"), Format::Png)
        .unwrap();
    r2.render_str("a", &dir.path().join("3.png"), Format::Png)
        .unwrap();
    let (w1, h1) = png_size(&fs::read(dir.path().join("1.png")).unwrap());
    let (w3, h3) = png_size(&fs::read(dir.path().join("3.png")).unwrap());
    assert!(
        w3.abs_diff(w1 * 3) <= 3 && h3.abs_diff(h1 * 3) <= 3,
        "{w1}x{h1} vs {w3}x{h3}"
    );
}

#[test]
fn render_auto_picks_png_from_extension() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("d.d2");
    fs::write(&src, "x -> y").unwrap();
    let out = dir.path().join("d.png");
    Renderer::new().render_auto(&src, &out).unwrap();
    png_size(&fs::read(out).unwrap());
}
