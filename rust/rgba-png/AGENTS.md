# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

RGBA8 frames and PNG files. slint-bindings uses it for screenshot tests
(`encode_png`, `decode_png`, `differing_pixels`, `diff_image`); scull uses
`read_png_header` to decode any PNG a terminal sends into RGBA under an
allocation cap. Apache-2.0, unlike the rest of the repo, because slint-bindings
is Apache-2.0. Not published; consumers pin a git rev.

## Rules

- `decode_png` returns samples as stored and `read_png_header` normalises to
  RGBA8. Keep them apart: screenshot tests want to see a non-RGBA baseline,
  scull wants every PNG.
- `read_png_header` reads only the header so a caller can refuse a frame by its
  size before any pixel buffer is allocated. Do not fold the two steps into one.
- A change to a function's output changes a consumer's baselines or images: say
  which consumer it affects in the commit.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
