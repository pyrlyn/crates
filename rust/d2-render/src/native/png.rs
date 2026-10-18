// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! SVG to PNG with the pure-Rust `resvg` rasterizer (feature `png`).

use std::path::Path;
use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{fontdb, Options, Tree};

use crate::{Error, Result};

/// System fonts, loaded once per process.
fn fonts() -> Arc<fontdb::Database> {
    static DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// Rasterize `svg` at `scale` (D2's default PNG scale is 2). Relative image
/// references resolve against `resources_dir`; remote `http(s)` images are
/// not fetched and are left out.
pub fn svg_to_png(svg: &str, scale: f64, resources_dir: Option<&Path>) -> Result<Vec<u8>> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(Error::Raster(format!("invalid scale {scale}")));
    }
    let opt = Options {
        resources_dir: resources_dir.map(Path::to_path_buf),
        fontdb: fonts(),
        ..Options::default()
    };
    let tree = Tree::from_str(svg, &opt).map_err(|e| Error::Raster(e.to_string()))?;
    let size = tree.size();
    let s = scale as f32;
    let w = (size.width() * s).ceil() as u32;
    let h = (size.height() * s).ceil() as u32;
    let mut pixmap = Pixmap::new(w.max(1), h.max(1))
        .ok_or_else(|| Error::Raster(format!("cannot allocate a {w}x{h} image")))?;
    resvg::render(&tree, Transform::from_scale(s, s), &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|e| Error::Raster(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_at_scale() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 20"><rect width="10" height="20" fill="red"/></svg>"#;
        let png = svg_to_png(svg, 2.0, None).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        // IHDR width/height.
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 20);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 40);
        assert!(svg_to_png(svg, 0.0, None).is_err());
        assert!(svg_to_png("not svg", 1.0, None).is_err());
    }
}
