// tests/ui.rs runs this with `allowed_logging_crates = ["tracing"]`, so `tracing` is the logging
// facade: its own items, the `tracing_core` items it re-exports when named through it, and items
// imported from it are fine. `log` is still reported.

use tracing::{Level, span};

#[tracing::instrument]
fn traced(n: u32) -> u32 {
    n + 1
}

fn main() {
    tracing::info!(answer = 42, "through the facade");
    // `Level` is defined in `tracing_core`.
    let _ = tracing::Level::INFO;
    let _ = Level::WARN;
    let work = span!(Level::DEBUG, "work");
    let _guard = work.enter();
    let _ = traced(1);

    log::info!("not the facade");
    let _ = log::max_level();
}
