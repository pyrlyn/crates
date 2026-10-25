// Paths resolve to the real crates, so re-exports (`tracing::Level` is `tracing_core::Level`)
// are matched by where they are defined.

use log::{Log, Metadata, Record};

struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }

    fn log(&self, _: &Record) {}

    fn flush(&self) {}
}

#[tracing::instrument]
fn traced(n: u32) -> u32 {
    n + 1
}

fn main() {
    log::info!("started");
    log::warn!(target: "app", "careful: {}", 1);
    let _ = log::max_level();
    let _ = log::set_logger(&StderrLogger);

    tracing::info!(answer = 42, "traced");
    let span = tracing::span!(tracing::Level::INFO, "work");
    let _guard = span.enter();
    let _ = traced(1);

    let _ = env_logger::builder().is_test(true).try_init();

    let drain = slog::Discard;
    let root = slog::Logger::root(drain, slog::o!());
    slog::info!(root, "slog message");

    quiet();
}

#[allow(lockstep_logging_only)]
fn quiet() {
    log::debug!("allowed");
}
