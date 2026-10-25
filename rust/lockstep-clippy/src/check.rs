//! What both lints share: a policy says which definitions are banned, and these functions find
//! them in macro expansions, resolved paths and method calls.

use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::macros::{macro_backtrace, span_is_local};
use clippy_utils::source::snippet_opt;
use rustc_data_structures::fx::FxHashSet;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{CrateNum, DefId};
use rustc_hir::{
    Expr, ExprKind, HirId, ItemKind, Node, Path, PathSegment, QPath, UseKind, UseTree,
};
use rustc_lint::{LateContext, Lint};
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_span::{ExpnId, Span, Symbol};

pub struct Policy {
    pub lint: &'static Lint,
    /// Whether a function, type, trait or other item is banned.
    pub item: fn(&LateContext<'_>, DefId) -> bool,
    /// Whether a macro is banned.
    pub macro_: fn(&LateContext<'_>, DefId) -> bool,
    /// The diagnostic message for a banned use, given how it is named.
    pub message: fn(&str) -> String,
    pub help: &'static str,
    /// Crates the policy lets through, as crate names with `_` for `-`: their own items, and
    /// banned items named through them (a facade's re-export, such as `logstep::log::info!`).
    pub allowed_crates: Vec<String>,
    /// Banned items the crate being linted imports with `use` through an allowed crate, filled
    /// by [`Policy::collect_allowed_imports`].
    pub allowed_imports: FxHashSet<DefId>,
}

impl Policy {
    fn bans_item(&self, cx: &LateContext<'_>, def_id: DefId) -> bool {
        (self.item)(cx, def_id) && !self.lets_through(cx, def_id)
    }

    fn bans_macro(&self, cx: &LateContext<'_>, def_id: DefId) -> bool {
        (self.macro_)(cx, def_id) && !self.lets_through(cx, def_id)
    }

    fn lets_through(&self, cx: &LateContext<'_>, def_id: DefId) -> bool {
        self.allows_crate(cx, def_id.krate) || self.allowed_imports.contains(&def_id)
    }

    pub fn allows_crate(&self, cx: &LateContext<'_>, krate: CrateNum) -> bool {
        let name = cx.tcx.crate_name(krate);
        self.allowed_crates
            .iter()
            .any(|allowed| allowed == name.as_str())
    }

    /// Whether a path goes through a module of an allowed crate (`logstep::log::Level`) or one
    /// imported through it (`use logstep::log;` then `log::Level`).
    fn named_through_allowed(&self, cx: &LateContext<'_>, segments: &[PathSegment<'_>]) -> bool {
        segments.iter().any(|segment| {
            matches!(segment.res, Res::Def(DefKind::Mod, id)
                if self.allows_crate(cx, id.krate) || self.allowed_imports.contains(&id))
        })
    }

    /// The macro call's path is not in the HIR, so read its first segment from the source:
    /// `logstep::log::info!(..)` and `#[logstep::instrument]` go through `logstep`.
    fn macro_named_through_allowed(&self, cx: &LateContext<'_>, span: Span) -> bool {
        let Some(text) = snippet_opt(cx, span) else {
            return false;
        };
        let text = text.trim_start_matches("#[").trim_start_matches("::");
        let first = text
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or_default();
        text[first.len()..].starts_with("::")
            && self.allowed_crates.iter().any(|allowed| allowed == first)
    }

    /// Records what the crate's `use` items import through an allowed crate: the named items, or
    /// for a glob import every item of the module.
    pub fn collect_allowed_imports(&mut self, cx: &LateContext<'_>) {
        if self.allowed_crates.is_empty() {
            return;
        }
        for item_id in cx.tcx.hir_free_items() {
            if let ItemKind::Use(tree) = cx.tcx.hir_item(item_id).kind {
                self.collect_use_tree(cx, &tree, false);
            }
        }
    }

    fn collect_use_tree(
        &mut self,
        cx: &LateContext<'_>,
        tree: &UseTree<'_>,
        through_allowed: bool,
    ) {
        let through_allowed =
            through_allowed || self.named_through_allowed(cx, tree.prefix.segments);
        if let UseKind::Nested { items } = tree.kind {
            for (item, _, _) in items {
                self.collect_use_tree(cx, item, through_allowed);
            }
            return;
        }
        if !through_allowed {
            return;
        }
        for res in tree.prefix.res.present_items() {
            match res {
                Res::Def(DefKind::Mod, module)
                    if matches!(tree.kind, UseKind::Glob) && !module.is_local() =>
                {
                    let children = cx.tcx.module_children(module);
                    self.allowed_imports
                        .extend(children.iter().filter_map(|child| child.res.opt_def_id()));
                }
                Res::Def(_, def_id) => {
                    self.allowed_imports.insert(def_id);
                }
                _ => {}
            }
        }
    }
}

/// One rule over definition paths as the compiler resolved them: the crate an item is defined
/// in (not one that re-exports it), then its first module, then its own name. `module: None`
/// bans the whole crate; empty `names` bans the whole module.
pub struct Rule {
    pub krate: &'static str,
    pub module: Option<&'static str>,
    pub names: &'static [&'static str],
}

impl Rule {
    pub const fn krate(krate: &'static str) -> Self {
        Self {
            krate,
            module: None,
            names: &[],
        }
    }

    pub const fn module(krate: &'static str, module: &'static str) -> Self {
        Self {
            krate,
            module: Some(module),
            names: &[],
        }
    }

    pub const fn items(
        krate: &'static str,
        module: &'static str,
        names: &'static [&'static str],
    ) -> Self {
        Self {
            krate,
            module: Some(module),
            names,
        }
    }

    fn matches(&self, path: &[Symbol]) -> bool {
        let Some((krate, rest)) = path.split_first() else {
            return false;
        };
        if krate.as_str() != self.krate {
            return false;
        }
        let Some(module) = self.module else {
            return true;
        };
        rest.first().is_some_and(|first| first.as_str() == module)
            && (self.names.is_empty()
                || rest
                    .last()
                    .is_some_and(|name| self.names.contains(&name.as_str())))
    }
}

pub fn matches_any(cx: &LateContext<'_>, rules: &[Rule], def_id: DefId) -> bool {
    let path = cx.get_def_path(def_id);
    rules.iter().any(|rule| rule.matches(&path))
}

/// Reports the outermost banned macro call that produced `span`, once per expansion. Calls a
/// macro from another crate generated are skipped: the user did not write them.
pub fn check_expansion(
    cx: &LateContext<'_>,
    policy: &Policy,
    seen: &mut FxHashSet<ExpnId>,
    span: Span,
) {
    let Some(call) = macro_backtrace(span)
        .filter(|call| policy.bans_macro(cx, call.def_id))
        .last()
    else {
        return;
    };
    if !seen.insert(call.expn)
        || !span_is_local(call.span)
        || policy.macro_named_through_allowed(cx, call.span)
    {
        return;
    }
    let name = format!("{}!", cx.tcx.item_name(call.def_id));
    report(cx, policy, call.span, &name);
}

/// Reports a resolved path (expression, type, pattern, trait bound or `impl ... for`) to a banned
/// item. `use` items are skipped so an import and its use are not both reported.
pub fn check_path(cx: &LateContext<'_>, policy: &Policy, path: &Path<'_>, hir_id: HirId) {
    let Res::Def(kind, mut def_id) = path.res else {
        return;
    };
    if !span_is_local(path.span) || is_use_item(cx, hir_id) {
        return;
    }
    if let DefKind::Ctor(..) = kind {
        def_id = cx.tcx.parent(def_id);
    }
    if policy.bans_item(cx, def_id) && !policy.named_through_allowed(cx, path.segments) {
        report(cx, policy, path.span, &display(cx, def_id));
    }
}

/// Reports calls to methods of banned traits (`v.par_iter()`, `<T>::par_iter(&v)`). Inherent
/// methods are left out: their type is reported where it is named. In a chain only the first
/// banned call is reported.
pub fn check_expr(
    cx: &LateContext<'_>,
    policy: &Policy,
    seen: &mut FxHashSet<ExpnId>,
    expr: &Expr<'_>,
) {
    check_expansion(cx, policy, seen, expr.span);
    if !span_is_local(expr.span) {
        return;
    }
    match expr.kind {
        ExprKind::MethodCall(segment, receiver, _, _) => {
            if let Some(def_id) = banned_trait_method(cx, policy, expr)
                && !is_banned_call(cx, policy, receiver)
            {
                report(
                    cx,
                    policy,
                    segment.ident.span.with_hi(expr.span.hi()),
                    &display(cx, def_id),
                );
            }
        }
        ExprKind::Path(QPath::TypeRelative(..)) => {
            if let Some(def_id) = banned_trait_method(cx, policy, expr) {
                report(cx, policy, expr.span, &display(cx, def_id));
            }
        }
        _ => {}
    }
}

fn banned_trait_method(cx: &LateContext<'_>, policy: &Policy, expr: &Expr<'_>) -> Option<DefId> {
    let def_id = match expr.kind {
        ExprKind::MethodCall(..) => cx.typeck_results().type_dependent_def_id(expr.hir_id)?,
        ExprKind::Path(ref qpath) => cx.qpath_res(qpath, expr.hir_id).opt_def_id()?,
        _ => return None,
    };
    let trait_id = cx.tcx.trait_of_assoc(def_id)?;
    policy.bans_item(cx, trait_id).then_some(def_id)
}

fn is_banned_call(cx: &LateContext<'_>, policy: &Policy, expr: &Expr<'_>) -> bool {
    match expr.peel_borrows().kind {
        ExprKind::MethodCall(..) => banned_trait_method(cx, policy, expr.peel_borrows()).is_some(),
        ExprKind::Call(callee, _) => {
            if let ExprKind::Path(ref qpath) = callee.kind
                && let Some(def_id) = cx.qpath_res(qpath, callee.hir_id).opt_def_id()
            {
                policy.bans_item(cx, def_id) || banned_trait_method(cx, policy, callee).is_some()
            } else {
                false
            }
        }
        _ => false,
    }
}

fn is_use_item(cx: &LateContext<'_>, hir_id: HirId) -> bool {
    match cx.tcx.hir_node(hir_id) {
        Node::Item(item) => matches!(item.kind, ItemKind::Use(..)),
        Node::NestedUseTree(_) => true,
        _ => false,
    }
}

fn display(cx: &LateContext<'_>, def_id: DefId) -> String {
    with_no_trimmed_paths!(cx.tcx.def_path_str(def_id))
}

fn report(cx: &LateContext<'_>, policy: &Policy, span: Span, name: &str) {
    span_lint_and_help(
        cx,
        policy.lint,
        span,
        (policy.message)(name),
        None,
        policy.help,
    );
}
