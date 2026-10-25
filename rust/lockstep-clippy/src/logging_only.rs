use crate::check::{self, Policy, Rule};
use rustc_data_structures::fx::FxHashSet;
use rustc_hir::def_id::{DefId, LOCAL_CRATE};
use rustc_hir::{Expr, HirId, Path};
use rustc_lint::{LateContext, LateLintPass, declare_lint, impl_lint_pass};
use rustc_span::ExpnId;

declare_lint! {
    /// ### What it does
    /// Flags logging that does not go through Lockstep: the `log`, `tracing`, `env_logger` and
    /// `slog` crates (macros, functions, types and traits) and std's `print!`, `println!`,
    /// `eprint!`, `eprintln!` and `dbg!`.
    ///
    /// The crates in `allowed_logging_crates` (default `lockstep` and `logstep`) are the logging
    /// facade: their items are fine, so are banned items named through them (a re-export such as
    /// `logstep::log::info!`, or `use logstep::log::info;`), and the lint does not run inside them.
    ///
    /// ### Configuration
    /// ```toml
    /// [lockstep_clippy]
    /// allowed_logging_crates = ["lockstep", "logstep"]
    /// ```
    ///
    /// ### Why is this bad?
    /// Only Lockstep may be used for logging, so every message has one format and one sink.
    ///
    /// ### Example
    /// ```rust,ignore
    /// log::info!("started");
    /// println!("started");
    /// ```
    pub LOCKSTEP_LOGGING_ONLY,
    Warn,
    "logging outside Lockstep"
}

/// Crates whose every item counts as logging. `tracing` re-exports items defined in
/// `tracing_core`, and `#[tracing::instrument]` comes from `tracing_attributes`.
const RULES: &[Rule] = &[
    Rule::krate("log"),
    Rule::krate("tracing"),
    Rule::krate("tracing_core"),
    Rule::krate("tracing_attributes"),
    Rule::krate("tracing_subscriber"),
    Rule::krate("env_logger"),
    Rule::krate("slog"),
    Rule::krate("slog_async"),
    Rule::krate("slog_scope"),
    Rule::krate("slog_stdlog"),
    Rule::krate("slog_term"),
];

fn is_banned(cx: &LateContext<'_>, def_id: DefId) -> bool {
    check::matches_any(cx, RULES, def_id)
}

fn is_banned_macro(cx: &LateContext<'_>, def_id: DefId) -> bool {
    // `dbg_macro` is not a pre-interned symbol, so compare names.
    cx.tcx.get_diagnostic_name(def_id).is_some_and(|name| {
        matches!(
            name.as_str(),
            "print_macro" | "println_macro" | "eprint_macro" | "eprintln_macro" | "dbg_macro"
        )
    }) || is_banned(cx, def_id)
}

/// The `dylint.toml` table this lint reads: dylint keys tables by library name.
pub const CONFIG_TABLE: &str = "lockstep_clippy";

#[derive(Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Crate names (`-` or `_`) that make up the logging facade.
    pub allowed_logging_crates: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            allowed_logging_crates: vec!["lockstep".to_owned(), "logstep".to_owned()],
        }
    }
}

impl Config {
    /// The first entry that is not a crate name, if any.
    pub fn invalid_crate(&self) -> Option<&str> {
        self.allowed_logging_crates
            .iter()
            .map(String::as_str)
            .find(|name| {
                name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            })
    }
}

pub struct LoggingOnly {
    policy: Policy,
    /// Set inside an allowed crate, which implements the logging this lint points to.
    skip: bool,
    seen: FxHashSet<ExpnId>,
}

impl LoggingOnly {
    pub fn new(config: &Config) -> Self {
        let allowed_crates = config
            .allowed_logging_crates
            .iter()
            .map(|name| name.replace('-', "_"))
            .collect();
        let policy = Policy {
            lint: LOCKSTEP_LOGGING_ONLY,
            item: is_banned,
            macro_: is_banned_macro,
            message: |name| format!("logging through `{name}`; only Lockstep may log"),
            help: "log through Lockstep instead",
            allowed_crates,
            allowed_imports: FxHashSet::default(),
        };
        Self {
            policy,
            skip: false,
            seen: FxHashSet::default(),
        }
    }
}

impl_lint_pass!(LoggingOnly => [LOCKSTEP_LOGGING_ONLY]);

impl<'tcx> LateLintPass<'tcx> for LoggingOnly {
    fn check_crate(&mut self, cx: &LateContext<'tcx>) {
        self.skip = self.policy.allows_crate(cx, LOCAL_CRATE);
        if !self.skip {
            self.policy.collect_allowed_imports(cx);
        }
    }

    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        if !self.skip {
            check::check_expr(cx, &self.policy, &mut self.seen, expr);
        }
    }

    fn check_path(&mut self, cx: &LateContext<'tcx>, path: &Path<'tcx>, hir_id: HirId) {
        if !self.skip {
            check::check_path(cx, &self.policy, path, hir_id);
        }
    }
}
