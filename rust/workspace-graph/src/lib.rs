//! A Cargo workspace's direct dependency graph and the rules a test asserts over it.
//!
//! [`Graph::load`] reads `cargo metadata --no-deps`: every workspace member and the
//! names of the packages it declares as dependencies, external ones included. Rules
//! come in two shapes: `check_*` returns the [`Violations`] as a value, so a test can
//! prove a rule fails on a synthetic graph built with [`FromIterator`], and `assert_*`
//! panics with one readable line per violation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use cargo_metadata::{DependencyKind, MetadataCommand};

/// Which declared dependencies count as edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `[dependencies]`, optional and target-specific ones included.
    Normal,
    /// `[dev-dependencies]`.
    Dev,
    /// `[build-dependencies]`.
    Build,
}

impl Kind {
    fn matches(self, kind: DependencyKind) -> bool {
        matches!(
            (self, kind),
            (Kind::Normal, DependencyKind::Normal)
                | (Kind::Dev, DependencyKind::Development)
                | (Kind::Build, DependencyKind::Build)
        )
    }
}

/// `cargo metadata` could not run or its output did not parse.
#[derive(Debug, thiserror::Error)]
#[error("cargo metadata failed: {0}")]
pub struct LoadError(#[source] cargo_metadata::Error);

/// Every broken rule, one message each, in a stable order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Violations(Vec<String>);

impl Violations {
    /// The messages, one per broken edge or missing member.
    pub fn messages(&self) -> &[String] {
        &self.0
    }

    fn into_result(self) -> Result<(), Violations> {
        if self.0.is_empty() { Ok(()) } else { Err(self) }
    }
}

impl fmt::Display for Violations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("\n"))
    }
}

impl std::error::Error for Violations {}

/// Workspace member name -> names of the packages it depends on directly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    deps: BTreeMap<String, BTreeSet<String>>,
}

impl Graph {
    /// Loads the workspace that contains `dir`, keeping dependencies of `kinds`.
    ///
    /// Declared dependencies do not depend on features, so optional ones always
    /// count: a feature-gated dependency is still one a build can pull in. Pass
    /// `env!("CARGO_MANIFEST_DIR")` from a test; `$CARGO` picks the same cargo.
    pub fn load(dir: impl AsRef<Path>, kinds: &[Kind]) -> Result<Graph, LoadError> {
        // `--no-deps` lists workspace members only, needs no network and no lockfile resolution.
        let meta = MetadataCommand::new()
            .current_dir(dir.as_ref())
            .no_deps()
            .exec()
            .map_err(LoadError)?;
        let deps = meta
            .workspace_packages()
            .into_iter()
            .map(|package| {
                let names = package
                    .dependencies
                    .iter()
                    .filter(|d| kinds.iter().any(|k| k.matches(d.kind)))
                    .map(|d| d.name.clone())
                    .collect();
                (package.name.to_string(), names)
            })
            .collect();
        Ok(Graph { deps })
    }

    /// Workspace member names, sorted.
    pub fn members(&self) -> impl Iterator<Item = &str> {
        self.deps.keys().map(String::as_str)
    }

    /// Direct dependencies of `member`, or `None` when it is not a member.
    pub fn deps(&self, member: &str) -> Option<&BTreeSet<String>> {
        self.deps.get(member)
    }

    /// Every member with its direct dependencies, for rules this crate does not cover.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &BTreeSet<String>)> {
        self.deps.iter().map(|(name, deps)| (name.as_str(), deps))
    }

    /// The same graph with external dependencies dropped: edges between members only.
    pub fn workspace_only(&self) -> Graph {
        let deps = self
            .deps
            .iter()
            .map(|(name, deps)| {
                let inner = deps
                    .iter()
                    .filter(|d| self.deps.contains_key(*d))
                    .cloned()
                    .collect();
                (name.clone(), inner)
            })
            .collect();
        Graph { deps }
    }

    /// No member outside `allowed` depends on any of `deps`. Names in `allowed`
    /// need not be members, so a planned owner is not an error.
    pub fn check_only_dependents(&self, deps: &[&str], allowed: &[&str]) -> Result<(), Violations> {
        let who = if allowed.is_empty() {
            "nothing may".to_owned()
        } else {
            format!("only {} may", allowed.join(", "))
        };
        let mut found = Vec::new();
        for (name, have) in self.iter().filter(|(n, _)| !allowed.contains(n)) {
            for dep in have.iter().filter(|d| deps.contains(&d.as_str())) {
                found.push(format!("{name} must not depend on {dep}; {who}"));
            }
        }
        Violations(found).into_result()
    }

    /// `member` depends on none of `banned`.
    pub fn check_forbidden(&self, member: &str, banned: &[&str]) -> Result<(), Violations> {
        let found = match self.deps(member) {
            None => vec![not_a_member(member)],
            Some(have) => have
                .iter()
                .filter(|d| banned.contains(&d.as_str()))
                .map(|dep| format!("{member} must not depend on {dep}"))
                .collect(),
        };
        Violations(found).into_result()
    }

    /// The direct dependencies of `member` are exactly `expected`, in any order.
    pub fn check_exact(&self, member: &str, expected: &[&str]) -> Result<(), Violations> {
        let Some(have) = self.deps(member) else {
            return Violations(vec![not_a_member(member)]).into_result();
        };
        let want: BTreeSet<&str> = expected.iter().copied().collect();
        let mut found: Vec<String> = have
            .iter()
            .filter(|d| !want.contains(d.as_str()))
            .map(|dep| format!("{member} depends on {dep}, which is not expected"))
            .collect();
        found.extend(
            want.iter()
                .filter(|d| !have.contains(**d))
                .map(|dep| format!("{member} does not depend on {dep}, which is expected")),
        );
        Violations(found).into_result()
    }

    /// Edges between members point down `layers` (index 0 is the bottom): a member
    /// depends only on members of its own layer or a lower one, and every member
    /// sits in a layer so a new crate has to be placed. Layer names that are not
    /// members are ignored, so planned crates may be listed early.
    pub fn check_layers(&self, layers: &[&[&str]]) -> Result<(), Violations> {
        let layer_of = |name: &str| layers.iter().position(|l| l.contains(&name));
        let mut found = Vec::new();
        for (name, have) in self.iter() {
            let Some(from) = layer_of(name) else {
                found.push(format!("{name} is in no layer"));
                continue;
            };
            for dep in have.iter().filter(|d| self.deps.contains_key(*d)) {
                if let Some(to) = layer_of(dep).filter(|to| *to > from) {
                    found.push(format!(
                        "{name} (layer {from}) must not depend on {dep} (layer {to})"
                    ));
                }
            }
        }
        Violations(found).into_result()
    }

    /// Panicking form of [`Graph::check_only_dependents`].
    #[track_caller]
    pub fn assert_only_dependents(&self, deps: &[&str], allowed: &[&str]) {
        fail_on(self.check_only_dependents(deps, allowed));
    }

    /// Panicking form of [`Graph::check_forbidden`].
    #[track_caller]
    pub fn assert_forbidden(&self, member: &str, banned: &[&str]) {
        fail_on(self.check_forbidden(member, banned));
    }

    /// Panicking form of [`Graph::check_exact`].
    #[track_caller]
    pub fn assert_exact(&self, member: &str, expected: &[&str]) {
        fail_on(self.check_exact(member, expected));
    }

    /// Panicking form of [`Graph::check_layers`].
    #[track_caller]
    pub fn assert_layers(&self, layers: &[&[&str]]) {
        fail_on(self.check_layers(layers));
    }
}

/// Builds a synthetic graph: each row is a member and its dependency names.
/// A dependency that is not itself a row counts as external.
impl<N, D> FromIterator<(N, D)> for Graph
where
    N: Into<String>,
    D: IntoIterator,
    D::Item: Into<String>,
{
    fn from_iter<I: IntoIterator<Item = (N, D)>>(rows: I) -> Self {
        let deps = rows
            .into_iter()
            .map(|(name, deps)| (name.into(), deps.into_iter().map(Into::into).collect()))
            .collect();
        Graph { deps }
    }
}

fn not_a_member(member: &str) -> String {
    format!("{member} is not a workspace member")
}

/// A test-support crate: a broken rule is a failed test, and the message is the report.
#[track_caller]
fn fail_on(result: Result<(), Violations>) {
    if let Err(violations) = result {
        panic!("\n{violations}\n");
    }
}
