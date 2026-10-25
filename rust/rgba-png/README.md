# rgba-png

RGBA8 frames and PNG files. A frame is tightly packed, row-major, 4 bytes per pixel.

| Function | What it does |
| --- | --- |
| `encode_png` | Frame to an RGBA8 PNG with an sRGB chunk |
| `decode_png` | PNG to `(width, height, samples)` as stored |
| `read_png_header` | Header of any PNG, with a cap on the decoder's buffers; `size()` before decoding, then `decode_rgba()` widens grey, grey+alpha, RGB, palette, low and 16-bit depths to RGBA8 |
| `channel_delta` | Largest channel difference between two pixels |
| `differing_pixels` | Pixels whose largest channel difference is above a threshold |
| `diff_image` | `left` faded, with differing pixels in opaque red |

```rust
let png = rgba_png::encode_png(1, 1, &[1, 2, 3, 255])?;
let header = rgba_png::read_png_header(&png, 64 << 20)?;
assert_eq!(header.size(), (1, 1));
assert_eq!(header.decode_rgba()?, [1, 2, 3, 255]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Licensed under Apache-2.0.
