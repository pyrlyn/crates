//! Render through the d2 executable (`$D2_BIN` or `d2` on `$PATH`): every
//! format, layout engine and D2 feature.
//!
//! cargo run -p d2-render --features cli --example cli -- [input.d2] [output.svg|png|pdf|txt]

use std::path::PathBuf;

use d2_render::{Error, Format, Layout, RenderOptions, Renderer};

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let input = args.next().map(PathBuf::from);
    let output = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "target/d2-cli-example.svg".into()),
    );
    let format = Format::from_path(&output)?;

    let renderer =
        Renderer::cli().options(RenderOptions::new().layout(Layout::Elk).theme(200).pad(20));
    match renderer.version() {
        Ok(v) => println!("using d2 {v}"),
        Err(e) if e.is_binary_not_found() => {
            eprintln!("{e}");
            std::process::exit(2);
        }
        Err(e) => return Err(e),
    }
    let result = match &input {
        Some(path) => renderer.render_file_report(path, &output, format),
        None => renderer.render_str_report(
            "direction: right\nclient -> server: request\nserver -> db: query\ndb: {shape: cylinder}",
            &output,
            format,
        ),
    };
    match result {
        Ok(report) => println!("{report}"),
        Err(Error::Syntax { diagnostics, .. }) => {
            for d in diagnostics {
                eprintln!("{d}");
            }
            std::process::exit(1);
        }
        Err(e) => return Err(e),
    }
    Ok(())
}
