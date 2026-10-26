//! target-prune: keep the last N cargo build runs (plus everything they depend on) in a
//! `target/` dir and drop the rest. Unix only (Linux, macOS): it reads cargo/rustc lock state
//! with flock/fcntl and inode metadata.

#[cfg(unix)]
mod app;

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    app::main()
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    eprintln!("target-prune supports only Unix (Linux, macOS)");
    std::process::ExitCode::FAILURE
}
