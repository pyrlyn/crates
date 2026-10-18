//! Render with the pure-Rust backend directly; no d2 binary needed.
//!
//! cargo run -p d2-render --example native -- [input.d2] [output.svg]

use d2_render::native::render_svg;
use d2_render::{RenderOptions, SvgInfo};

const SHOWCASE: &str = "vars: {
  d2-config: {theme-id: 3}
  owner: Platform team
}
classes: {
  store: {shape: cylinder; style.multiple: true}
}
direction: right
title: Checkout (${owner}) {near: top-center; shape: text; style.font-size: 24}
user: {shape: person}
app: App {
  api -> worker: jobs
}
cache.class: store
db.class: store
user -> app.api: HTTPS {target-arrowhead.shape: diamond}
app.worker -> cache
app.worker -> db: {source-arrowhead: 1; target-arrowhead: {shape: cf-many}}
";

fn main() -> Result<(), d2_render::Error> {
    let mut args = std::env::args().skip(1);
    let source = match args.next() {
        Some(path) => std::fs::read_to_string(path)?,
        None => SHOWCASE.to_string(),
    };
    let output = args
        .next()
        .unwrap_or_else(|| "target/d2-native-example.svg".into());
    let out = render_svg(&source, &RenderOptions::default())?;
    for w in &out.warnings {
        eprintln!("warning: {w}");
    }
    let info = SvgInfo::parse(&out.svg);
    if let Some(dir) = std::path::Path::new(&output).parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&output, &out.svg)?;
    println!("wrote {output} ({info})");
    Ok(())
}
