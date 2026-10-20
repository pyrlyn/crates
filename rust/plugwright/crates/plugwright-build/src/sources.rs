//! What a crate's build reads from disk, found from its manifests without
//! running cargo: the path packages it depends on (through `path`,
//! `workspace = true`, `[patch]`, `[replace]` and config `paths`), the
//! workspace manifests, the lockfile and every cargo config file that applies.
//!
//! The result over-approximates: optional, platform-specific and patched
//! packages count whether or not the build uses them. An extra package costs
//! a cache miss; a missing one would serve a stale artifact.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use toml::{Table, Value};

/// Inputs of one build besides the toolchain and the environment.
#[derive(Debug, Default)]
pub(crate) struct Sources {
    /// Package directories; every file under them is an input.
    pub packages: BTreeSet<PathBuf>,
    /// Single files elsewhere: workspace manifests and cargo configs. Some
    /// do not exist; creating one changes the build.
    pub files: BTreeSet<PathBuf>,
    /// The workspace's `Cargo.lock`, which the build itself may create or update.
    pub lockfile: PathBuf,
}

/// Dependency tables that reach the built library (dev-dependencies do not).
const DEP_TABLES: [&str; 3] = ["dependencies", "build-dependencies", "build_dependencies"];

pub(crate) fn sources(crate_dir: &Path) -> Result<Sources, String> {
    let crate_dir = normalize(crate_dir);
    let mut r = Reader::default();
    let mut s = Sources::default();
    let mut queue = vec![crate_dir.clone()];

    // The workspace that builds the crate owns the lockfile, `[patch]` and `[replace]`.
    let manifest = crate_dir.join("Cargo.toml");
    let own = r.read(&manifest)?.ok_or_else(|| format!("{} not found", manifest.display()))?;
    let (root, root_manifest) = r.workspace_of(&crate_dir, &own)?.unwrap_or((crate_dir.clone(), own));
    s.files.insert(root.join("Cargo.toml"));
    s.lockfile = root.join("Cargo.lock");
    for patches in root_manifest.get("patch").and_then(Value::as_table).into_iter().flat_map(Table::values) {
        push_paths(patches, &root, &mut queue);
    }
    if let Some(replace) = root_manifest.get("replace") {
        push_paths(replace, &root, &mut queue);
    }

    // Config files from the crate directory up, then `$CARGO_HOME`: cargo runs
    // in the crate directory. Their relative paths start at the directory that
    // holds `.cargo`.
    let home = cargo_home();
    let dirs = crate_dir.ancestors().map(|d| d.join(".cargo")).chain(home);
    for cfg in dirs.flat_map(|d| [d.join("config.toml"), d.join("config")]) {
        s.files.insert(cfg.clone());
        let Some(t) = r.read(&cfg)? else { continue };
        let base = cfg.parent().and_then(Path::parent).unwrap_or(Path::new("/")).to_owned();
        for patches in t.get("patch").and_then(Value::as_table).into_iter().flat_map(Table::values) {
            push_paths(patches, &base, &mut queue);
        }
        for p in t.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            queue.push(base.join(p));
        }
    }

    while let Some(dir) = queue.pop() {
        let dir = normalize(&dir);
        if !s.packages.insert(dir.clone()) {
            continue;
        }
        let manifest = dir.join("Cargo.toml");
        let Some(m) = r.read(&manifest)? else {
            // cargo will fail on it; until it exists, its absence is the input.
            s.files.insert(manifest);
            continue;
        };
        let ws = r.workspace_of(&dir, &m)?;
        if let Some((root, _)) = &ws {
            s.files.insert(root.join("Cargo.toml"));
        }
        let targets = m.get("target").and_then(Value::as_table).into_iter().flat_map(Table::values);
        let holders = std::iter::once(&*m).chain(targets.filter_map(Value::as_table));
        for table in holders.flat_map(|h| DEP_TABLES.iter().filter_map(|t| h.get(*t)?.as_table())) {
            for (name, spec) in table {
                if let Some(p) = spec.get("path").and_then(Value::as_str) {
                    queue.push(dir.join(p));
                } else if spec.get("workspace").and_then(Value::as_bool) == Some(true)
                    && let Some((root, wm)) = &ws
                    && let Some(p) = lookup(wm, &["workspace", "dependencies", name, "path"]).and_then(Value::as_str)
                {
                    queue.push(root.join(p));
                }
            }
        }
    }
    Ok(s)
}

/// `path` of every `name = { path = "…" }` in a `[patch.<source>]` or `[replace]` table.
fn push_paths(table: &Value, base: &Path, queue: &mut Vec<PathBuf>) {
    let specs = table.as_table().into_iter().flat_map(Table::values);
    queue.extend(specs.filter_map(|v| v.get("path")?.as_str()).map(|p| base.join(p)));
}

fn lookup<'a>(t: &'a Table, keys: &[&str]) -> Option<&'a Value> {
    let (first, rest) = keys.split_first()?;
    rest.iter().try_fold(t.get(*first)?, |v, k| v.get(*k))
}

fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| std::env::home_dir().map(|h| h.join(".cargo")))
}

/// Canonical when the path exists (so `a/../b` and `b` are one package),
/// lexically cleaned when it does not.
fn normalize(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Parsed TOML files, each read once.
#[derive(Default)]
struct Reader {
    parsed: HashMap<PathBuf, Option<Rc<Table>>>,
}

impl Reader {
    /// `None` when the file does not exist; a file that does not parse is an
    /// error (cargo would refuse it too).
    fn read(&mut self, path: &Path) -> Result<Option<Rc<Table>>, String> {
        if let Some(t) = self.parsed.get(path) {
            return Ok(t.clone());
        }
        let t = match std::fs::read_to_string(path) {
            Ok(text) => Some(Rc::new(text.parse::<Table>().map_err(|e| format!("{}: {e}", path.display()))?)),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => None,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        self.parsed.insert(path.to_owned(), t.clone());
        Ok(t)
    }

    /// The workspace `dir`'s package belongs to, if any: `package.workspace`,
    /// else the nearest ancestor manifest with `[workspace]` that does not
    /// exclude it. A package with its own `[workspace]` is its own root.
    fn workspace_of(&mut self, dir: &Path, manifest: &Rc<Table>) -> Result<Option<(PathBuf, Rc<Table>)>, String> {
        if manifest.contains_key("workspace") {
            return Ok(Some((dir.to_owned(), manifest.clone())));
        }
        if let Some(p) = lookup(manifest, &["package", "workspace"]).and_then(Value::as_str) {
            let root = normalize(&dir.join(p));
            return Ok(self.read(&root.join("Cargo.toml"))?.map(|t| (root, t)));
        }
        for anc in dir.ancestors().skip(1) {
            let Some(t) = self.read(&anc.join("Cargo.toml"))? else { continue };
            let Some(ws) = t.get("workspace") else { continue };
            let excluded = ws
                .get("exclude")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|e| dir.starts_with(normalize(&anc.join(e))));
            return Ok((!excluded).then(|| (anc.to_owned(), t)));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, files: &[(&str, &str)]) {
        for (path, text) in files {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
    }

    fn names(root: &Path, set: &BTreeSet<PathBuf>) -> Vec<String> {
        let root = normalize(root);
        // Joined with `/` so the expectations read the same on Windows, where `display` uses `\`.
        set.iter()
            .filter_map(|p| p.strip_prefix(&root).ok())
            .map(|p| p.iter().map(|c| c.to_string_lossy()).collect::<Vec<_>>().join("/"))
            .collect()
    }

    const PKG: &str = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n";

    #[test]
    fn path_dependencies_are_followed_through_every_table_but_dev() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        write(
            d,
            &[
                (
                    "plugin/Cargo.toml",
                    "[package]\nname = \"p\"\nversion = \"0.1.0\"\n[workspace]\n\
                     [dependencies]\na = { path = \"../a\" }\nserde = \"1\"\n\
                     [build-dependencies]\nb = { path = \"../b\" }\n\
                     [target.'cfg(windows)'.dependencies]\nw = { path = \"../w\" }\n\
                     [dev-dependencies]\nt = { path = \"../t\" }\n",
                ),
                ("a/Cargo.toml", &format!("{PKG}[dependencies]\nc = {{ path = \"./sub/../../c\" }}\n")),
                ("b/Cargo.toml", PKG),
                ("c/Cargo.toml", PKG),
                ("w/Cargo.toml", PKG),
                ("t/Cargo.toml", PKG),
            ],
        );
        let s = sources(&d.join("plugin")).unwrap();
        assert_eq!(names(d, &s.packages), ["a", "b", "c", "plugin", "w"]);
        assert_eq!(s.lockfile, normalize(d).join("plugin/Cargo.lock"));
    }

    #[test]
    fn workspace_inheritance_patches_and_config_paths_are_followed() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        write(
            d,
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"alone\"]\n\
                     [workspace.dependencies]\nshared = { path = \"libs/shared\" }\nlog = \"0.4\"\n\
                     [patch.crates-io]\nserde = { path = \"vendor/serde\" }\n\
                     [replace]\n\"foo:1.0.0\" = { path = \"vendor/foo\" }\n",
                ),
                (
                    "crates/plugin/Cargo.toml",
                    &format!("{PKG}[dependencies]\nshared.workspace = true\nlog.workspace = true\n"),
                ),
                ("libs/shared/Cargo.toml", PKG),
                ("vendor/serde/Cargo.toml", PKG),
                ("vendor/foo/Cargo.toml", PKG),
                ("crates/plugin/.cargo/config.toml", "paths = [\"local/override\"]\n"),
                ("alone/Cargo.toml", PKG),
            ],
        );
        let s = sources(&d.join("crates/plugin")).unwrap();
        assert_eq!(
            names(d, &s.packages),
            ["crates/plugin", "crates/plugin/local/override", "libs/shared", "vendor/foo", "vendor/serde"]
        );
        let files = names(d, &s.files);
        assert_eq!(s.lockfile, normalize(d).join("Cargo.lock"));
        for f in ["Cargo.toml", "crates/plugin/.cargo/config.toml", ".cargo/config.toml"] {
            assert!(files.iter().any(|x| x == f), "{f} not in {files:?}");
        }
        assert!(
            s.files.contains(&normalize(d).join("crates/plugin/local/override/Cargo.toml")),
            "missing manifests count"
        );

        // An excluded package is its own workspace root.
        let s = sources(&d.join("alone")).unwrap();
        assert_eq!(names(d, &s.packages), ["alone"]);
        assert_eq!(s.lockfile, normalize(d).join("alone/Cargo.lock"));
    }

    #[test]
    fn package_workspace_key_names_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        write(
            d,
            &[
                ("ws/Cargo.toml", "[workspace]\n[workspace.dependencies]\nz = { path = \"../z\" }\n"),
                (
                    "elsewhere/p/Cargo.toml",
                    &format!("{PKG}workspace = \"../../ws\"\n[dependencies]\nz.workspace = true\n"),
                ),
                ("z/Cargo.toml", PKG),
            ],
        );
        let s = sources(&d.join("elsewhere/p")).unwrap();
        assert_eq!(names(d, &s.packages), ["elsewhere/p", "z"]);
        assert_eq!(s.lockfile, normalize(d).join("ws/Cargo.lock"));
    }

    #[test]
    fn broken_or_missing_manifests_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sources(dir.path()).unwrap_err().contains("Cargo.toml not found"));
        std::fs::write(dir.path().join("Cargo.toml"), "[package\n").unwrap();
        assert!(sources(dir.path()).unwrap_err().contains("Cargo.toml"));
    }
}
