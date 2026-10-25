# rgba-png — completed tasks

### T1. Extract the PNG helpers from slint-bindings and scull

`encode_png`, `decode_png`, `channel_delta`, `differing_pixels` and `diff_image` come
unchanged from slint-bindings' `pixel-diff` crate (T72). `read_png_header` and
`PngHeader::{size, decode_rgba}` come from scull's `scull-image` `raster::decode_png`
and `widen`: same `png::Limits`, same `normalize_to_color8`, same widening, split in
two so scull keeps checking the size before the pixel buffer is allocated. weft's
screenshot helpers stay in weft: they compare frames of different sizes and paint
the diff differently. Tests cover every color type, palette, 2-bit and 16-bit depth,
the allocation cap and corrupt input. Adoption is T2 and T3.

### T2. Adopt rgba-png in scull

https://github.com/pyrlyn/scull/pull/46 (e19e7ed). `scull-image`'s `raster::decode_png`
now calls `read_png_header(data, MAX_IMAGE_BYTES)`, checks `rgba_len` on `size()`
before `decode_rgba()` and maps png errors through `corrupt`; its `widen` is gone and
`png` is a dev-dependency only. The crate is pinned to a6f89b2. Checked with scull's
workspace tests (plus a new truncated-PNG test), clippy, fmt and hakari.

### T3. Adopt rgba-png in slint-bindings

https://github.com/pyrlyn/slint-bindings/pull/42 (5e8fae1). `slintx-wgpu` and
`slint-bindings-ffi` take the crate as a dev-dependency pinned to a6f89b2, and
`crates/pixel-diff` is deleted. Checked with slint-bindings' nextest (131/131 on
macOS, the snapshot e2e tests also pass on Windows CI), clippy, fmt and the slintx
license check.
