//! Watch a file or directory and re-render on every change (Ctrl-C to stop).
//!
//! cargo run -p d2-render --features watch --example watch -- diagrams/

use std::path::PathBuf;

use d2_render::watch::{watch, WatchConfig, WatchEvent};
use d2_render::Renderer;

fn main() -> Result<(), d2_render::Error> {
    let target = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let config = WatchConfig {
        render_on_start: true,
        ..WatchConfig::default()
    };
    let _handle = watch(
        Renderer::new(),
        std::slice::from_ref(&target),
        config,
        |event| match event {
            WatchEvent::Rendered {
                result: Ok(report), ..
            } => println!("{report}"),
            WatchEvent::Rendered {
                input,
                result: Err(e),
                ..
            } => {
                eprintln!("{}: {e}", input.display());
                for d in e.diagnostics() {
                    eprintln!("  {d}");
                }
            }
            WatchEvent::WatchError(e) => eprintln!("watcher: {e}"),
        },
    )?;
    println!("watching {} (Ctrl-C to stop)", target.display());
    loop {
        std::thread::park();
    }
}
