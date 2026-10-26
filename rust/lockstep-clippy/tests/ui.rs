//! `ui/` holds the std-only cases, one file per lint, plus `lockstep.rs` and `logstep.rs`, whose
//! crate names are in the default `allowed_logging_crates` (and `lockstep` also skips the
//! concurrency lint). `examples/` holds the cases that need the real `log`, `tracing`, `rayon`,
//! ... crates: Cargo builds them with this package's dev-dependencies, so their paths resolve to
//! the actual definitions. `allowed_logging_crates` runs one of them with its own `dylint.toml`.

/// The library name dylint matches against the built file (`liblockstep_clippy@<toolchain>`).
const LIBRARY: &str = "lockstep_clippy";

#[test]
fn ui() {
    // An absolute path: compiletest replaces the directory name with `$DIR` anywhere in the output,
    // so a relative `ui` would turn `Builder` into `B$DIRlder`.
    dylint_testing::ui_test(
        LIBRARY,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui"),
    );
}

#[test]
fn logging_crates() {
    dylint_testing::ui_test_example(LIBRARY, "logging_crates");
}

#[test]
fn concurrency_crates() {
    dylint_testing::ui_test_example(LIBRARY, "concurrency_crates");
}

#[test]
fn allowed_logging_crates() {
    dylint_testing::ui::Test::example(LIBRARY, "allowed_logging_crates")
        .dylint_toml("[lockstep_clippy]\nallowed_logging_crates = [\"tracing\"]\n")
        .run();
}
