use crate::check::{self, Policy, Rule};
use rustc_data_structures::fx::FxHashSet;
use rustc_hir::def_id::{DefId, LOCAL_CRATE};
use rustc_hir::{Expr, HirId, Path};
use rustc_lint::{LateContext, LateLintPass, declare_lint, impl_lint_pass};
use rustc_span::ExpnId;

declare_lint! {
    /// ### What it does
    /// Flags explicit threads and locks: `std::thread::{spawn, scope, Builder}`,
    /// `std::sync::{Mutex, RwLock, Condvar, Barrier}`, crossbeam's threads, channels and sync
    /// types, rayon (including parallel iterators), parking_lot, spin and
    /// `tokio::sync::{Mutex, RwLock, Semaphore, Barrier, Notify}`. The `lockstep` crate itself is
    /// not checked.
    ///
    /// ### Why is this bad?
    /// Lockstep keeps one global lock order so locks cannot deadlock. A thread or lock taken
    /// outside it can break that order.
    ///
    /// ### Example
    /// ```rust,ignore
    /// let counter = std::sync::Mutex::new(0);
    /// std::thread::spawn(|| {});
    /// ```
    pub LOCKSTEP_EXPLICIT_CONCURRENCY,
    Warn,
    "explicit threads or locks outside Lockstep"
}

/// Paths are where items are defined: `crossbeam::scope` is `crossbeam_utils::thread::scope`,
/// `parking_lot::Mutex` methods live in `lock_api`, std's `Mutex` in `std::sync::poison`.
const RULES: &[Rule] = &[
    Rule::items("std", "thread", &["spawn", "scope", "Builder"]),
    Rule::items("std", "sync", &["Mutex", "RwLock", "Condvar", "Barrier"]),
    Rule::krate("crossbeam_channel"),
    Rule::module("crossbeam_utils", "thread"),
    Rule::module("crossbeam_utils", "sync"),
    Rule::krate("rayon"),
    Rule::krate("rayon_core"),
    Rule::krate("parking_lot"),
    Rule::krate("parking_lot_core"),
    Rule::krate("lock_api"),
    Rule::krate("spin"),
    Rule::items(
        "tokio",
        "sync",
        &["Mutex", "RwLock", "Semaphore", "Barrier", "Notify"],
    ),
];

fn is_banned(cx: &LateContext<'_>, def_id: DefId) -> bool {
    check::matches_any(cx, RULES, def_id)
}

pub struct ExplicitConcurrency {
    policy: Policy,
    /// Set for the `lockstep` crate, which implements what this lint points to.
    skip: bool,
    seen: FxHashSet<ExpnId>,
}

impl Default for ExplicitConcurrency {
    fn default() -> Self {
        let policy = Policy {
            lint: LOCKSTEP_EXPLICIT_CONCURRENCY,
            item: is_banned,
            macro_: is_banned,
            message: |name| format!("explicit concurrency through `{name}`"),
            help: "take threads and locks through Lockstep instead",
            allowed_crates: Vec::new(),
            allowed_imports: FxHashSet::default(),
        };
        Self {
            policy,
            skip: false,
            seen: FxHashSet::default(),
        }
    }
}

impl_lint_pass!(ExplicitConcurrency => [LOCKSTEP_EXPLICIT_CONCURRENCY]);

impl<'tcx> LateLintPass<'tcx> for ExplicitConcurrency {
    fn check_crate(&mut self, cx: &LateContext<'tcx>) {
        self.skip = cx.tcx.crate_name(LOCAL_CRATE).as_str() == "lockstep";
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
