//! Render a diagram and wrap it in a standalone HTML page.
//!
//! cargo run -p d2-render --features web --example web_page

use std::path::Path;

use d2_render::web::{data_uri_file, write_html_page};
use d2_render::{Format, Renderer};

fn main() -> Result<(), d2_render::Error> {
    let svg = Path::new("target/d2-web-example.svg");
    Renderer::new().render_str("browser -> server -> db", svg, Format::Svg)?;
    write_html_page("Request flow", svg, Path::new("target/d2-web-example.html"))?;
    let uri = data_uri_file(svg)?;
    println!(
        "wrote target/d2-web-example.html; data URI is {} bytes",
        uri.len()
    );
    Ok(())
}
