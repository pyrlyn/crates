//! Find a package of a cargo workspace by name, for building a crate the user
//! names on the command line rather than one a manifest points at.
//!
//! `cargo metadata` is asked rather than the manifests read directly (as
//! [`crate::sources`] does): member globs, `exclude` and renamed libraries are
//! cargo's rules, and this runs once per command, not on every cache lookup.

use std::path::{Path, PathBuf};

use cargo_metadata::{MetadataCommand, TargetKind};
use plugwright_core::error::{PluginError, Result};

/// A workspace package with a library target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// The directory holding its `Cargo.toml`.
    pub dir: PathBuf,
    /// The library target's name (`my_crate` for package `my-crate`).
    pub lib_name: String,
    /// The workspace's cargo target directory.
    pub target_dir: PathBuf,
}

/// Library kinds `cargo rustc --crate-type` can turn into any other library;
/// a proc-macro cannot be.
const LIBRARY: [TargetKind; 5] =
    [TargetKind::Lib, TargetKind::RLib, TargetKind::DyLib, TargetKind::CDyLib, TargetKind::StaticLib];

/// The package `name` of the workspace that contains `dir`, run with `cargo`.
pub fn find_package(cargo: &Path, dir: &Path, name: &str) -> Result<Package> {
    let fail = |message: String| PluginError::build(name, message);
    let meta = MetadataCommand::new()
        .cargo_path(cargo)
        .current_dir(dir)
        .no_deps()
        .exec()
        .map_err(|e| fail(format!("cargo metadata in {}: {e}", dir.display())))?;
    let members = meta.workspace_packages();
    let Some(pkg) = members.iter().find(|p| p.name.as_str() == name) else {
        let mut names: Vec<&str> = members.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        return Err(fail(format!(
            "no package `{name}` in the workspace at {} (packages: {})",
            meta.workspace_root,
            names.join(", ")
        )));
    };
    let lib = pkg
        .targets
        .iter()
        .find(|t| LIBRARY.iter().any(|k| t.is_kind(k.clone())))
        .ok_or_else(|| fail(format!("package `{name}` has no library target (a proc-macro or binaries only)")))?;
    let dir = pkg.manifest_path.parent().ok_or_else(|| fail(format!("bad manifest path {}", pkg.manifest_path)))?;
    Ok(Package {
        name: name.to_owned(),
        dir: dir.into(),
        lib_name: lib.name.clone(),
        target_dir: meta.target_directory.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn package(dir: &Path, name: &str, body: &str) {
        write(
            dir,
            "Cargo.toml",
            &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{body}"),
        );
    }

    fn workspace() -> tempfile::TempDir {
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path();
        write(root, "Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n");
        package(&root.join("crates/my-lib"), "my-lib", "");
        write(root, "crates/my-lib/src/lib.rs", "");
        package(&root.join("crates/renamed"), "renamed", "[lib]\nname = \"other\"\ncrate-type = [\"cdylib\"]\n");
        write(root, "crates/renamed/src/lib.rs", "");
        package(&root.join("crates/tool"), "tool", "");
        write(root, "crates/tool/src/main.rs", "fn main() {}\n");
        package(&root.join("crates/derive"), "derive", "[lib]\nproc-macro = true\n");
        write(root, "crates/derive/src/lib.rs", "");
        ws
    }

    fn cargo() -> PathBuf {
        std::env::var_os("CARGO").map_or_else(|| "cargo".into(), PathBuf::from)
    }

    #[test]
    fn finds_a_member_by_name_from_any_directory_inside() {
        let ws = workspace();
        let root = std::fs::canonicalize(ws.path()).unwrap();
        let p = find_package(&cargo(), &root.join("crates/renamed/src"), "my-lib").unwrap();
        assert_eq!(p.name, "my-lib");
        assert_eq!(std::fs::canonicalize(&p.dir).unwrap(), root.join("crates/my-lib"));
        assert_eq!(p.lib_name, "my_lib");
        assert_eq!(std::fs::canonicalize(p.target_dir.parent().unwrap()).unwrap(), root);
        assert_eq!(find_package(&cargo(), &root, "renamed").unwrap().lib_name, "other");
    }

    #[test]
    fn unknown_names_and_crates_without_a_library_are_refused() {
        let ws = workspace();
        let err = |name: &str| match find_package(&cargo(), ws.path(), name) {
            Err(PluginError::Build { id, message }) => {
                assert_eq!(id, name);
                message
            }
            other => panic!("{name}: {other:?}"),
        };
        let m = err("missing");
        assert!(m.contains("no package `missing`") && m.contains("packages: derive, my-lib, renamed, tool"), "{m}");
        assert!(err("tool").contains("no library target"));
        assert!(err("derive").contains("no library target"));
        // Packages that are dependencies, not members, do not count either.
        assert!(err("cargo_metadata").contains("no package"));
    }

    #[test]
    fn a_directory_outside_any_workspace_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            find_package(&cargo(), dir.path(), "x"),
            Err(PluginError::Build { message, .. }) if message.contains("cargo metadata")
        ));
    }
}
