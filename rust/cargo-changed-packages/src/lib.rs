//! Workspace packages affected by a set of changed paths.
//!
//! Paths are matched to workspace members by `determinator` (the guppy project's crate for
//! this), which also adds every reverse dependency of a changed package.
#![deny(missing_docs)]

mod cache;

use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use determinator::Determinator;
use guppy::graph::{DependencyDirection, PackageGraph};

pub use cache::CacheStatus;

/// Why the affected packages could not be computed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `cargo metadata` failed or its output could not be understood.
    #[error("cannot read the cargo workspace: {0}")]
    Metadata(#[from] Box<guppy::Error>),
    /// `cargo metadata` could not be run, or exited unsuccessfully.
    #[error("{0}")]
    Cargo(String),
    /// Cargo resolved a different workspace root than the one requested, so the changed
    /// paths would be matched against the wrong directory.
    #[error("{requested} is not the workspace root (cargo reports {actual})")]
    NotWorkspaceRoot {
        /// The directory passed to [`affected`].
        requested: PathBuf,
        /// The workspace root cargo found.
        actual: PathBuf,
    },
    /// The requested or the reported workspace root does not resolve to a canonical path,
    /// so the two cannot be compared.
    #[error("cannot resolve {path}")]
    Canonicalize {
        /// The path that failed to resolve.
        path: PathBuf,
        /// Why it failed.
        #[source]
        source: std::io::Error,
    },
}

/// The workspace packages affected by a change.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Affected {
    /// Workspace package names that changed or depend, directly or not, on a changed
    /// package. Empty when [`all`](Self::all) is set: check `all` first.
    pub packages: BTreeSet<String>,
    /// The answer could not be computed exactly, so every package must be treated as
    /// affected.
    pub all: bool,
}

impl Affected {
    fn everything() -> Self {
        Self {
            packages: BTreeSet::new(),
            all: true,
        }
    }

    /// A nextest filterset selecting the affected packages, for `cargo nextest run -E`.
    ///
    /// `None` means no filter is needed: run everything. An empty set gives `none()`.
    /// Package names use the `=` (equality) matcher because the default is a glob and
    /// the reference says to always prefix programmatically built expressions; operators
    /// and `none()` are defined in the filterset reference at
    /// <https://nexte.st/docs/filtersets/reference/> (checked 2026-10-05).
    pub fn nextest_filter(&self) -> Option<String> {
        if self.all {
            return None;
        }
        if self.packages.is_empty() {
            return Some("none()".to_owned());
        }
        let terms: Vec<String> = self
            .packages
            .iter()
            .map(|p| format!("package(={p})"))
            .collect();
        Some(terms.join(" | "))
    }
}

/// Computes the affected packages of a workspace, loading the package graph at most once.
///
/// Create one per run and call [`affected`](Self::affected) for every gate: `cargo metadata`
/// runs lazily on the first call that needs the graph and its result is reused. With a cache
/// directory the metadata JSON is also kept on disk between runs (see [`CacheStatus`]).
pub struct Resolver {
    root: PathBuf,
    cargo: OsString,
    cache_dir: Option<PathBuf>,
    graph: OnceCell<PackageGraph>,
    status: OnceCell<CacheStatus>,
}

impl Resolver {
    /// A resolver for the workspace at `workspace_root`, using `cargo` from `PATH` and no
    /// disk cache.
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            root: workspace_root.to_path_buf(),
            cargo: OsString::from("cargo"),
            cache_dir: None,
            graph: OnceCell::new(),
            status: OnceCell::new(),
        }
    }

    /// Use this cargo executable instead of `cargo` from `PATH`.
    #[must_use]
    pub fn cargo(mut self, program: impl Into<OsString>) -> Self {
        self.cargo = program.into();
        self
    }

    /// Keep the metadata JSON in this directory between runs.
    #[must_use]
    pub fn cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(dir.into());
        self
    }

    /// What the cache did, once the graph has been loaded; `None` before that, which also
    /// means `cargo metadata` has not run.
    pub fn cache_status(&self) -> Option<&CacheStatus> {
        self.status.get()
    }

    /// The workspace packages that `changed` affects, reverse dependencies included.
    ///
    /// `changed` paths are relative to the workspace root (repo-relative as
    /// `git-changed-paths` produces them is the same thing when the workspace sits at the
    /// repository top level). A path outside every package (a root `README.md`) affects
    /// nothing.
    ///
    /// The old package graph would need a checkout of the base ref, so a change to anything
    /// that alters the graph or the build (`Cargo.toml`, `Cargo.lock`, `.cargo/config*`,
    /// `rust-toolchain*`, `build.rs`) returns `all: true` instead of guessing. So does a
    /// path that cannot be mapped to the workspace (non-UTF-8, absolute elsewhere, or
    /// escaping it). In those cases, and when `changed` is empty, cargo is not run.
    ///
    /// # Errors
    ///
    /// Fails when `cargo metadata` cannot describe the workspace, the root is not the
    /// workspace root, or either root does not resolve, so they cannot be compared.
    pub fn affected(&self, changed: &[PathBuf]) -> Result<Affected, Error> {
        let Some(relative) = relative_paths(&self.root, changed) else {
            return Ok(Affected::everything());
        };
        if relative.iter().any(|p| alters_build(p)) {
            return Ok(Affected::everything());
        }
        if relative.is_empty() {
            return Ok(Affected::default());
        }
        let graph = self.graph()?;
        // Both graphs are the same: the base graph is unavailable, and manifest changes
        // already returned above, so only file-to-package matching and reverse dependencies
        // remain.
        let mut determinator = Determinator::new(graph, graph);
        determinator.add_changed_paths(&relative);
        let set = determinator.compute();
        let packages = set
            .affected_set
            .packages(DependencyDirection::Forward)
            .filter(|p| p.in_workspace())
            .map(|p| p.name().to_owned())
            .collect();
        Ok(Affected {
            packages,
            all: false,
        })
    }

    fn graph(&self) -> Result<&PackageGraph, Error> {
        if let Some(graph) = self.graph.get() {
            return Ok(graph);
        }
        let (graph, status) = self.load()?;
        // std::fs::canonicalize resolves symlinks such as macOS's /tmp, which cargo also does.
        // A path that does not resolve is an error: two failures must not compare equal.
        let canonical = |path: &Path| {
            path.canonicalize().map_err(|source| Error::Canonicalize {
                path: path.to_path_buf(),
                source,
            })
        };
        let actual = graph.workspace().root();
        if canonical(&self.root)? != canonical(actual.as_std_path())? {
            return Err(Error::NotWorkspaceRoot {
                requested: self.root.clone(),
                actual: actual.as_std_path().to_path_buf(),
            });
        }
        let _ = self.status.set(status);
        Ok(self.graph.get_or_init(|| graph))
    }

    fn load(&self) -> Result<(PackageGraph, CacheStatus), Error> {
        let entry = self
            .cache_dir
            .as_deref()
            .and_then(|dir| cache::Entry::open(dir, &self.root, &self.cargo));
        if let Some(entry) = &entry
            && let Some(json) = entry.read()
            && let Ok(graph) = PackageGraph::from_json(&json)
        {
            return Ok((graph, CacheStatus::Hit));
        }
        let json = self.run_metadata()?;
        let graph = PackageGraph::from_json(&json).map_err(Box::new)?;
        let status = match entry {
            None => CacheStatus::Disabled,
            Some(entry) => {
                let outside = outside_manifests(&graph, &self.root);
                match entry.write(&json, &outside) {
                    Ok(()) => CacheStatus::Miss,
                    Err(why) => CacheStatus::MissNotStored(why),
                }
            }
        };
        Ok((graph, status))
    }

    fn run_metadata(&self) -> Result<String, Error> {
        let out = Command::new(&self.cargo)
            .args(["metadata", "--format-version", "1"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| Error::Cargo(format!("cannot run cargo metadata: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Cargo(format!(
                "cargo metadata failed ({}): {}",
                out.status,
                stderr.trim()
            )));
        }
        String::from_utf8(out.stdout)
            .map_err(|_| Error::Cargo("cargo metadata printed non-UTF-8 output".to_owned()))
    }
}

/// Manifests of local (path) packages that sit outside `root`; the cache key cannot see them.
fn outside_manifests(graph: &PackageGraph, root: &Path) -> Vec<PathBuf> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    graph
        .packages()
        .filter(|p| p.source().is_path())
        .map(|p| p.manifest_path().as_std_path().to_path_buf())
        .filter(|path| {
            !path
                .canonicalize()
                .unwrap_or_else(|_| path.clone())
                .starts_with(&root)
        })
        .collect()
}

/// Computes the workspace packages that `changed` affects, reverse dependencies included.
///
/// A one-shot [`Resolver`] without a cache; see [`Resolver::affected`] for the rules.
///
/// # Errors
///
/// Fails when `cargo metadata` cannot describe the workspace or `workspace_root` is not
/// the workspace root.
pub fn affected(workspace_root: &Path, changed: &[PathBuf]) -> Result<Affected, Error> {
    Resolver::new(workspace_root).affected(changed)
}

/// Normalises to `/`-separated workspace-relative strings; `None` when any path cannot be.
fn relative_paths(root: &Path, changed: &[PathBuf]) -> Option<Vec<String>> {
    changed
        .iter()
        .map(|path| {
            let path = if path.is_absolute() {
                path.strip_prefix(root).ok()?
            } else {
                path.as_path()
            };
            let mut parts = Vec::new();
            for component in path.components() {
                match component {
                    Component::Normal(part) => parts.push(part.to_str()?),
                    Component::CurDir => {}
                    _ => return None,
                }
            }
            Some(parts.join("/"))
        })
        .collect()
}

fn alters_build(path: &str) -> bool {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    matches!(name, "Cargo.toml" | "Cargo.lock" | "build.rs")
        || name.starts_with("rust-toolchain")
        || (name.starts_with("config") && (dir == ".cargo" || dir.ends_with("/.cargo")))
}
