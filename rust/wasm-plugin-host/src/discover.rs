// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Plugin discovery. Finds user plugins under
//! `<home>/plugins/<id>/versions/<digest12>/` (following each `current`
//! file) and project plugins under `<project>/.<app>/plugins/<id>/`. The
//! application parses its own manifest through [`Manifest`]; this module
//! checks the id, refuses a wasm path that leaves the package, and computes
//! the digest. A user plugin wins an id clash with a notice: a repository
//! must not shadow a package the user installed. Discovery answers `Loaded`
//! or `Skipped` and never fails the process.
//!
//! A user plugin's `link` pointer wins over `current`. Its target is read
//! in place, and the plugin is reported as `dev`.

use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::digest::package_digest;
use crate::layout::{self, LINK};

/// The manifest file every package carries.
pub const MANIFEST_FILE: &str = "plugin.toml";

/// What an application parses out of [`MANIFEST_FILE`]. Validation that is
/// specific to that application lives in [`Manifest::parse`]; the id match
/// and the wasm-path check live here so every host applies them once.
pub trait Manifest: Sized {
    /// Reads and validates `manifest_path`. `package_dir` is the directory
    /// the package will be digested from.
    fn parse(package_dir: &Path, manifest_path: &Path) -> Result<Self, String>;

    /// The id the manifest claims.
    fn id(&self) -> &str;

    /// Relative path of the WebAssembly module, when the package has one.
    /// Absolute paths and `..` components are refused.
    fn wasm(&self) -> Option<&str>;
}

/// Where a plugin's directory was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `<home>/plugins/<id>/versions/<digest12>/`, following `current`.
    User,
    /// `<project>/.<app>/plugins/<id>/`, read in place.
    Project,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Source::User => "user",
            Source::Project => "project",
        })
    }
}

/// One discovered plugin id, loaded or skipped with why.
#[derive(Debug, Clone)]
pub struct Plugin<M> {
    /// The directory name it was found under.
    pub id: String,
    /// User or project.
    pub source: Source,
    /// The package directory: the version directory for a staged user
    /// plugin, or the `link` target when [`Plugin::dev`] is true.
    pub dir: PathBuf,
    /// The outcome of parsing, validating and digesting it.
    pub state: State<M>,
    /// True when a `link` pointer names the directory. The package is read
    /// in place and its bytes change on every rebuild, so an application
    /// that stores a grant should not key that grant on [`State`]'s digest.
    pub dev: bool,
}

/// A discovered plugin's manifest state.
#[derive(Debug, Clone)]
pub enum State<M> {
    /// The manifest parsed and the package digested.
    Loaded {
        /// The parsed manifest.
        manifest: M,
        /// SHA-256 over the package tree ([`package_digest`](crate::package_digest)).
        digest: String,
    },
    /// Missing, unreadable or invalid. Shown, never fatal.
    Skipped {
        /// Why, for the application's plugin list.
        reason: String,
    },
}

/// One discovery pass: every plugin found, plus shadowing notices.
#[derive(Debug)]
pub struct Discovered<M> {
    /// Every plugin id found, sorted by id.
    pub plugins: Vec<Plugin<M>>,
    /// Notices such as a project plugin shadowed by a user one.
    pub notices: Vec<String>,
}

impl<M> Default for Discovered<M> {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
            notices: Vec::new(),
        }
    }
}

/// Scans `<home>/plugins` and, when `project_root` is given,
/// `<project_root>/.<app>/plugins`. An id present in both keeps the user
/// plugin and drops the project one with a notice.
///
/// `app` is the single path segment inside `.<app>` (no leading dot). An
/// empty root is not an error.
pub fn discover<M: Manifest>(home: &Path, app: &str, project_root: Option<&Path>) -> Discovered<M> {
    let mut out = Discovered::default();
    let user_root = home.join("plugins");
    for id in scan_ids(&user_root) {
        let plugin_dir = user_root.join(&id);
        let (dir, dev) = match resolve_user_dir(&plugin_dir) {
            Ok(resolved) => resolved,
            Err(reason) => {
                out.plugins.push(Plugin {
                    id,
                    source: Source::User,
                    dir: plugin_dir,
                    state: State::Skipped { reason },
                    dev: false,
                });
                continue;
            }
        };
        out.plugins.push(load_one(
            id,
            Source::User,
            &dir,
            &dir.join(MANIFEST_FILE),
            dev,
        ));
    }
    match project_plugins_root(app, project_root) {
        Ok(Some(project_root_dir)) => {
            for id in scan_ids(&project_root_dir) {
                if out.plugins.iter().any(|p| p.id == id) {
                    out.notices.push(format!(
                        "project plugin {id:?} is shadowed by the user plugin of the same id"
                    ));
                    continue;
                }
                let dir = project_root_dir.join(&id);
                out.plugins.push(load_one(
                    id,
                    Source::Project,
                    &dir,
                    &dir.join(MANIFEST_FILE),
                    false,
                ));
            }
        }
        Ok(None) => {}
        Err(reason) => out.notices.push(reason),
    }
    out.plugins.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Parses and validates one manifest, then digests `dir`. `expect_id` checks
/// the manifest against a directory name for a discovered plugin (`Some`);
/// an installer that does not have a directory name yet passes `None` and
/// reuses this same parse, wasm check and digest.
pub fn load_package<M: Manifest>(
    dir: &Path,
    manifest_path: &Path,
    expect_id: Option<&str>,
) -> Result<(M, String), String> {
    let manifest = M::parse(dir, manifest_path)?;
    if let Some(id) = expect_id
        && manifest.id() != id
    {
        return Err(format!(
            "plugin.toml id {:?} does not match directory {id:?}",
            manifest.id()
        ));
    }
    if let Some(wasm) = manifest.wasm()
        && !wasm_path_is_safe(wasm)
    {
        return Err(format!(
            "wasm {wasm:?} must be a path inside the package directory"
        ));
    }
    let digest = package_digest(dir).map_err(|e| format!("cannot digest package: {e}"))?;
    Ok((manifest, digest))
}

/// Where a user plugin's package sits: the `link` target when one is
/// present, else the version `current` points at. A link takes priority, so
/// a development pointer does not require removing the staged version first.
fn resolve_user_dir(plugin_dir: &Path) -> Result<(PathBuf, bool), String> {
    match layout::read_pointer(plugin_dir, LINK) {
        Ok(Some(path)) => Ok((PathBuf::from(path), true)),
        Ok(None) => current_version_dir(plugin_dir).map(|dir| (dir, false)),
        Err(e) => Err(format!(
            "cannot read {}: {e}",
            plugin_dir.join(LINK).display()
        )),
    }
}

/// Directory names directly under `dir`. Empty (never an error) when `dir`
/// itself does not exist, so an unused plugin root is not a notice.
fn scan_ids(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        // No valid id starts with `.`; `plugins/.staging/` holds downloads
        // that have not been installed yet.
        .filter(|name| !name.starts_with('.'))
        .collect();
    ids.sort();
    ids
}

fn project_plugins_root(app: &str, project_root: Option<&Path>) -> Result<Option<PathBuf>, String> {
    let Some(root) = project_root else {
        return Ok(None);
    };
    if !app_is_safe(app) {
        return Err(format!(
            "application name {app:?} is not a single path segment"
        ));
    }
    Ok(Some(root.join(format!(".{app}")).join("plugins")))
}

fn app_is_safe(app: &str) -> bool {
    !app.is_empty() && !app.contains(['/', '\\', '\0', '.']) && app != ".."
}

/// Follows a user plugin's `current` file to its version directory. The
/// pointer must be a hex version id, so a tampered `current` cannot leave
/// `versions/`.
fn current_version_dir(plugin_dir: &Path) -> Result<PathBuf, String> {
    let current = plugin_dir.join(layout::CURRENT);
    let digest12 = fs::read_to_string(&current)
        .map_err(|e| format!("cannot read {}: {e}", current.display()))?;
    let digest12 = digest12.trim();
    if digest12.is_empty() {
        return Err(format!("{} is empty", current.display()));
    }
    if !layout::is_version_id(digest12) {
        return Err(format!("{} is not a version id", current.display()));
    }
    Ok(plugin_dir.join("versions").join(digest12))
}

fn load_one<M: Manifest>(
    id: String,
    source: Source,
    dir: &Path,
    manifest_path: &Path,
    dev: bool,
) -> Plugin<M> {
    let state = load_package(dir, manifest_path, Some(&id))
        .map(|(manifest, digest)| State::Loaded { manifest, digest })
        .unwrap_or_else(|reason| State::Skipped { reason });
    Plugin {
        id,
        source,
        dir: dir.to_path_buf(),
        state,
        dev,
    }
}

fn wasm_path_is_safe(wasm: &str) -> bool {
    let path = Path::new(wasm);
    !path.is_absolute()
        && path
            .components()
            .all(|c| !matches!(c, Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str =
        "api = 1\nid = \"demo\"\nversion = \"0.1.0\"\nname = \"Demo\"\nwasm = \"plugin.wasm\"\n";

    /// The fields discovery itself looks at. Parsing a real manifest is the
    /// application's job; this double only feeds the checks in this module.
    #[derive(Debug)]
    struct Demo {
        id: String,
        wasm: Option<String>,
    }

    impl Manifest for Demo {
        fn parse(_package_dir: &Path, manifest_path: &Path) -> Result<Self, String> {
            let text = fs::read_to_string(manifest_path)
                .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
            if text.contains("not_a_field") {
                return Err(format!("{}: unknown field", manifest_path.display()));
            }
            let id = quoted_field(&text, "id")
                .ok_or_else(|| format!("{}: missing id", manifest_path.display()))?;
            Ok(Self {
                id,
                wasm: quoted_field(&text, "wasm"),
            })
        }

        fn id(&self) -> &str {
            &self.id
        }

        fn wasm(&self) -> Option<&str> {
            self.wasm.as_deref()
        }
    }

    fn quoted_field(text: &str, key: &str) -> Option<String> {
        let prefix = format!("{key} = \"");
        let line = text.lines().find(|line| line.starts_with(&prefix))?;
        let rest = line[prefix.len()..].trim_end();
        rest.strip_suffix('"').map(str::to_string)
    }

    fn write(dir: &Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn digest_is_stable_and_changes_when_any_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "plugin.toml", MANIFEST);
        write(dir.path(), "nested/plugin.wasm", "one");
        let first = package_digest(dir.path()).unwrap();
        assert_eq!(
            first, "240c323dca8fb8757a4340e3f938928ff63ceac1be1ceaad499be1b280ef7b9b",
            "the byte layout of the digest is part of the package identity"
        );
        assert_eq!(first, package_digest(dir.path()).unwrap());

        write(dir.path(), "nested/plugin.wasm", "two");
        assert_ne!(first, package_digest(dir.path()).unwrap());
    }

    #[test]
    fn empty_tree_digests_to_sha256_of_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            package_digest(dir.path()).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn user_plugin_shadows_project_plugin_with_notice() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let user_dir = home.path().join("plugins/demo");
        write(&user_dir, "current", "abc123456789");
        write(&user_dir, "versions/abc123456789/plugin.toml", MANIFEST);
        write(&user_dir, "versions/abc123456789/plugin.wasm", "user");
        write(
            &repo.path().join(".app/plugins/demo"),
            "plugin.toml",
            MANIFEST,
        );

        let found = discover::<Demo>(home.path(), "app", Some(repo.path()));

        assert_eq!(found.plugins.len(), 1, "{found:?}");
        assert_eq!(found.plugins[0].source, Source::User);
        assert!(
            found.notices.iter().any(|n| n.contains("demo")),
            "{:?}",
            found.notices
        );
    }

    #[test]
    fn malformed_manifest_is_listed_as_skipped() {
        let repo = tempfile::tempdir().unwrap();
        write(
            &repo.path().join(".app/plugins/bad"),
            "plugin.toml",
            "id = \"bad\"\nnot_a_field = true\n",
        );

        let found = discover::<Demo>(repo.path(), "app", Some(repo.path()));

        assert_eq!(found.plugins.len(), 1);
        assert!(matches!(&found.plugins[0].state, State::Skipped { reason } if !reason.is_empty()));
    }

    #[test]
    fn wasm_path_escaping_package_dir_is_skipped() {
        let repo = tempfile::tempdir().unwrap();
        let escaping = MANIFEST.replace("plugin.wasm", "../../etc/passwd");
        write(
            &repo.path().join(".app/plugins/demo"),
            "plugin.toml",
            &escaping,
        );

        let found = discover::<Demo>(repo.path(), "app", Some(repo.path()));

        assert_eq!(found.plugins.len(), 1);
        assert!(
            matches!(&found.plugins[0].state, State::Skipped { reason } if reason.contains("wasm"))
        );
    }

    #[test]
    fn staging_directory_is_not_a_plugin_id() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join("plugins/.staging/versions/abc123456789"),
            "plugin.toml",
            MANIFEST,
        );
        write(
            &home.path().join("plugins/.staging"),
            "current",
            "abc123456789",
        );

        let found = discover::<Demo>(home.path(), "app", None);

        assert!(found.plugins.is_empty(), "{found:?}");
        assert!(found.notices.is_empty());
    }

    #[test]
    fn tampered_current_pointer_is_skipped() {
        let home = tempfile::tempdir().unwrap();
        let plugin = home.path().join("plugins/demo");
        write(&plugin, "current", "../../etc");
        write(&plugin, "plugin.toml", MANIFEST);

        let found = discover::<Demo>(home.path(), "app", None);

        assert_eq!(found.plugins.len(), 1);
        assert!(matches!(
            &found.plugins[0].state,
            State::Skipped { reason } if reason.contains("version id")
        ));
    }

    /// A `link` pointer wins over a staged `current` and the plugin reports
    /// `dev`. The digest still tracks the linked bytes, which is why a grant
    /// for a development plugin must not be keyed on it.
    #[test]
    fn link_pointer_wins_over_current() {
        let home = tempfile::tempdir().unwrap();
        let staged = home.path().join("plugins/demo/versions/abc123456789");
        write(&staged, "plugin.toml", MANIFEST);
        write(&staged, "plugin.wasm", "staged");
        write(&home.path().join("plugins/demo"), "current", "abc123456789");

        let dev_dir = tempfile::tempdir().unwrap();
        write(dev_dir.path(), "plugin.toml", MANIFEST);
        write(dev_dir.path(), "plugin.wasm", "one");
        layout::link(&home.path().join("plugins/demo"), dev_dir.path()).unwrap();

        let found = discover::<Demo>(home.path(), "app", None);
        assert_eq!(found.plugins.len(), 1, "{found:?}");
        let plugin = &found.plugins[0];
        assert!(plugin.dev, "{plugin:?}");
        assert_eq!(plugin.dir, dev_dir.path());
        let State::Loaded { digest, .. } = &plugin.state else {
            panic!("loaded: {plugin:?}");
        };

        write(dev_dir.path(), "plugin.wasm", "two");
        let rebuilt = discover::<Demo>(home.path(), "app", None);
        let State::Loaded {
            digest: rebuilt_digest,
            ..
        } = &rebuilt.plugins[0].state
        else {
            panic!("reloaded");
        };
        assert_ne!(digest, rebuilt_digest);
        assert!(rebuilt.plugins[0].dev);
    }
}
