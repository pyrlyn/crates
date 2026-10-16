use std::fs;
use std::path::{Path, PathBuf};

use cargo_changed_packages::{Affected, Error, affected};
use tempfile::TempDir;

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// `a` depends on `b`; `c` is unrelated.
fn fixture() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\", \"c\"]\nresolver = \"2\"\n",
    );
    for (name, deps) in [("a", "b = { path = \"../b\" }\n"), ("b", ""), ("c", "")] {
        write(
            root,
            &format!("{name}/Cargo.toml"),
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}"
            ),
        );
        write(root, &format!("{name}/src/lib.rs"), "");
    }
    write(root, "README.md", "docs\n");
    dir
}

fn run(dir: &TempDir, changed: &[&str]) -> Affected {
    let changed: Vec<PathBuf> = changed.iter().map(PathBuf::from).collect();
    affected(dir.path(), &changed).unwrap()
}

fn names(a: &Affected) -> Vec<&str> {
    a.packages.iter().map(String::as_str).collect()
}

#[test]
fn change_in_dependency_selects_it_and_its_dependents() {
    let a = run(&fixture(), &["b/src/lib.rs"]);
    assert_eq!(names(&a), ["a", "b"]);
    assert!(!a.all);
}

#[test]
fn change_in_leaf_dependent_selects_only_it() {
    let a = run(&fixture(), &["a/src/lib.rs"]);
    assert_eq!(names(&a), ["a"]);
}

#[test]
fn change_in_unrelated_package_selects_only_it() {
    let a = run(&fixture(), &["c/src/lib.rs"]);
    assert_eq!(names(&a), ["c"]);
    assert!(!a.all);
}

#[test]
fn root_readme_selects_nothing() {
    let a = run(&fixture(), &["README.md"]);
    assert!(a.packages.is_empty());
    assert!(!a.all);
}

#[test]
fn no_changes_select_nothing() {
    let a = run(&fixture(), &[]);
    assert_eq!(a, Affected::default());
}

#[test]
fn several_changes_union() {
    let a = run(&fixture(), &["c/src/lib.rs", "b/src/lib.rs", "README.md"]);
    assert_eq!(names(&a), ["a", "b", "c"]);
}

#[test]
fn build_altering_paths_select_all_without_running_cargo() {
    // A missing root proves cargo is not consulted: it would fail on it.
    let missing = Path::new("/nonexistent/workspace");
    for path in [
        "Cargo.lock",
        "Cargo.toml",
        "b/Cargo.toml",
        ".cargo/config.toml",
        ".cargo/config",
        "b/.cargo/config.toml",
        "rust-toolchain.toml",
        "rust-toolchain",
        "b/build.rs",
    ] {
        let a = affected(
            missing,
            &[PathBuf::from("c/src/lib.rs"), PathBuf::from(path)],
        )
        .unwrap();
        assert!(a.all, "{path} must select all");
        assert!(a.packages.is_empty(), "{path}");
    }
}

#[test]
fn lookalike_names_do_not_select_all() {
    let dir = fixture();
    for path in ["c/src/config.toml", "c/src/my_build.rs", "c/Cargo.toml.md"] {
        assert!(!run(&dir, &[path]).all, "{path}");
    }
}

#[test]
fn unmappable_paths_select_all() {
    let dir = fixture();
    for path in ["../outside/x.rs", "/elsewhere/x.rs"] {
        assert!(run(&dir, &[path]).all, "{path}");
    }
}

#[test]
fn absolute_path_inside_the_root_is_accepted() {
    let dir = fixture();
    let inside = dir.path().join("c/src/lib.rs");
    let a = affected(dir.path(), &[inside]).unwrap();
    assert_eq!(names(&a), ["c"]);
}

#[test]
fn root_that_is_not_the_workspace_root_is_an_error() {
    let dir = fixture();
    let err = affected(&dir.path().join("b"), &[PathBuf::from("src/lib.rs")]).unwrap_err();
    assert!(matches!(err, Error::NotWorkspaceRoot { .. }), "{err}");
}

#[test]
fn directory_without_a_workspace_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = affected(dir.path(), &[PathBuf::from("x.rs")]).unwrap_err();
    assert!(matches!(err, Error::Cargo(_)), "{err}");
}

#[test]
fn nextest_filter_formats() {
    let set = |names: &[&str]| Affected {
        packages: names.iter().map(|n| (*n).to_owned()).collect(),
        all: false,
    };
    assert_eq!(
        set(&["b", "a"]).nextest_filter().unwrap(),
        "package(=a) | package(=b)"
    );
    assert_eq!(set(&["a"]).nextest_filter().unwrap(), "package(=a)");
    assert_eq!(set(&[]).nextest_filter().unwrap(), "none()");
    let all = Affected {
        all: true,
        ..Affected::default()
    };
    assert_eq!(all.nextest_filter(), None);
}
