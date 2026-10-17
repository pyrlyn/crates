#![allow(dead_code)]

use std::path::PathBuf;

#[cfg(feature = "cli")]
use d2_render::Renderer;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// `Some(renderer)` when a real d2 binary is available, else prints why the
/// calling test is skipped.
#[cfg(feature = "cli")]
pub fn real_d2(test: &str) -> Option<Renderer> {
    let r = Renderer::cli();
    match r.version() {
        Ok(_) => Some(r),
        Err(e) => {
            eprintln!("SKIP {test}: d2 not available ({e})");
            None
        }
    }
}

pub fn is_svg(bytes: &[u8]) -> bool {
    let s = String::from_utf8_lossy(bytes);
    let s = s.trim_start();
    s.starts_with("<?xml") || s.starts_with("<svg")
}
