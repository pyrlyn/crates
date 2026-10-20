# rgba-png

Plan for the `rgba-png` crate of https://github.com/pyrlyn/crates: RGBA8 frames and
PNG files shared by scull and slint-bindings.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |
| T3 | todo | P2 | 2 | 0% | |

### T2. Adopt rgba-png in scull

Replace `decode_png` and `widen` in `crates/scull-image/src/raster.rs` with
`read_png_header(data, MAX_IMAGE_BYTES)`, keep `rgba_len` on `size()` before
`decode_rgba()` and map png errors through `corrupt` as today. Pin the crate by git rev.
Needs this crate on `main`. Done when scull's suite is green on the crate, its own
copy is gone, and scull's `toolchain.md` lists the crate.

### T3. Adopt rgba-png in slint-bindings

Replace `crates/pixel-diff` (added by slint-bindings T72) with this crate in the
dev-dependencies of `slintx-wgpu` and `slint-bindings-ffi`, pinned by git rev, and
delete `crates/pixel-diff`. Needs T72 merged there. Done when slint-bindings' suite is
green on the crate and its `toolchain.md` lists it.
