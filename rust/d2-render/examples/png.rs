//! Render PNG in pure Rust (native backend + resvg); no d2 binary needed.
//!
//! cargo run -p d2-render --features png --example png -- [input.d2] [output.png]

use std::path::PathBuf;

use d2_render::{Error, Format, RenderOptions, Renderer};

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let input = args.next().map(PathBuf::from);
    let output = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "target/d2-example.png".into()),
    );

    // `scale` is the pixel ratio (D2's default for PNG is 2).
    let renderer = Renderer::new().options(RenderOptions::new().theme(4).scale(2.0));
    let report = match &input {
        Some(path) => renderer.render_file_report(path, &output, Format::Png)?,
        None => renderer.render_str_report(
            "direction: right\nweb -> api: REST\napi -> db\ndb: {shape: cylinder}",
            &output,
            Format::Png,
        )?,
    };
    println!("{report}");
    Ok(())
}
