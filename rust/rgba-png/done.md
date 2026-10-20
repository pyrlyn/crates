# rgba-png — completed tasks

### T1. Extract the PNG helpers from slint-bindings and scull

`encode_png`, `decode_png`, `channel_delta`, `differing_pixels` and `diff_image` come
unchanged from slint-bindings' `pixel-diff` crate (T72). `read_png_header` and
`PngHeader::{size, decode_rgba}` come from scull's `scull-image` `raster::decode_png`
and `widen`: same `png::Limits`, same `normalize_to_color8`, same widening, split in
two so scull keeps checking the size before the pixel buffer is allocated. weft's
screenshot helpers stay in weft: they compare frames of different sizes and paint
the diff differently. Tests cover every color type, palette, 2-bit and 16-bit depth,
the allocation cap and corrupt input. Adoption is T2 and T3 in `plan.md`.
