//! Render a diagram with the default (pure-Rust) backend and print the report.
//!
//! cargo run -p d2-render --example basic -- [input.d2] [output.svg]
//!
//! Add `--features png` for `.png` output, or see the `cli` example for the
//! d2 executable.

use std::path::PathBuf;

use d2_render::{Error, Format, RenderOptions, Renderer};

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let input = args.next().map(PathBuf::from);
    let output = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "target/d2-example.svg".into()),
    );
    let format = Format::from_path(&output)?;

    let renderer = Renderer::new().options(RenderOptions::new().theme(0).pad(20));
    println!("using {}", renderer.version()?);

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
