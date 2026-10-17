//! Standalone runner for `RunnerCommand::Path`. Hosts that can re-exec
//! themselves call `plugwright::maybe_run_runner()` instead.

fn main() {
    std::process::exit(plugwright_isolate::run_runner());
}
