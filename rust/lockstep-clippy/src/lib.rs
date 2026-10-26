#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_data_structures;
extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod check;
mod explicit_concurrency;
mod logging_only;

dylint_linting::dylint_library!();

#[allow(clippy::no_mangle_with_rust_abi)]
#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, lint_store: &mut rustc_lint::LintStore) {
    dylint_linting::init_config(sess);
    let table = logging_only::CONFIG_TABLE;
    let config = dylint_linting::config::<logging_only::Config>(table)
        .unwrap_or_else(|err| sess.dcx().fatal(format!("dylint.toml `[{table}]`: {err}")))
        .unwrap_or_default();
    if let Some(name) = config.invalid_crate() {
        sess.dcx().fatal(format!(
            "dylint.toml `[{table}]`: `{name}` in `allowed_logging_crates` is not a crate name"
        ));
    }

    lint_store.register_lints(&[
        logging_only::LOCKSTEP_LOGGING_ONLY,
        explicit_concurrency::LOCKSTEP_EXPLICIT_CONCURRENCY,
    ]);
    lint_store.register_late_lint_pass(Box::new(move |_| {
        Box::new(logging_only::LoggingOnly::new(&config))
    }));
    lint_store.register_late_lint_pass(Box::new(|_| {
        Box::<explicit_concurrency::ExplicitConcurrency>::default()
    }));
}
