//! The metadata cache and the "load the graph once" rule. A shim stands in for `cargo`: it
//! logs every call, then runs the real cargo, so the tests count `cargo metadata` runs
//! instead of guessing from timings. They need a POSIX `sh`.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use cargo_changed_packages::{CacheStatus, Error, Resolver};
use tempfile::TempDir;

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn manifest(name: &str, deps: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}"
    )
}

/// A workspace in `ws/`: `a` depends on `b`; `c` is unrelated; `a` also depends on `ext`,
/// a path package outside the workspace root. The lock file exists.
struct Fixture {
    dir: TempDir,
    cache: TempDir,
    shim: PathBuf,
    log: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "ws/Cargo.toml",
            "[workspace]\nmembers = [\"a\", \"b\", \"c\"]\nresolver = \"2\"\n",
        );
        let deps = "b = { path = \"../b\" }\next = { path = \"../../ext\" }\n";
        write(root, "ws/a/Cargo.toml", &manifest("a", deps));
        write(root, "ws/b/Cargo.toml", &manifest("b", ""));
        write(root, "ws/c/Cargo.toml", &manifest("c", ""));
        write(root, "ext/Cargo.toml", &manifest("ext", ""));
        for src in ["ws/a", "ws/b", "ws/c", "ext"] {
            write(root, &format!("{src}/src/lib.rs"), "");
        }
        let real = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = Command::new(&real)
            .args(["generate-lockfile", "--offline"])
            .current_dir(root.join("ws"))
            .status()
            .unwrap();
        assert!(status.success());
        let log = root.join("cargo.log");
        let shim = root.join("cargo-shim");
        // CARGO_SHIM_VERSION fakes a different cargo version; CARGO_SHIM_FAIL breaks metadata.
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\n\
             if [ \"$1\" = --version ] && [ -n \"$CARGO_SHIM_VERSION\" ]; then echo \"$CARGO_SHIM_VERSION\"; exit 0; fi\n\
             exec '{real}' \"$@\"\n",
            log = log.display(),
        );
        fs::write(&shim, script).unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            dir,
            cache: tempfile::tempdir().unwrap(),
            shim,
            log,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("ws")
    }

    fn resolver(&self) -> Resolver {
        Resolver::new(&self.root())
            .cargo(&self.shim)
            .cache_dir(self.cache.path())
    }

    /// How many times the shim ran `cargo metadata`.
    fn metadata_runs(&self) -> usize {
        fs::read_to_string(&self.log).map_or(0, |log| {
            log.lines().filter(|l| l.starts_with("metadata")).count()
        })
    }

    /// Load the graph through a fresh resolver and return the cache status.
    fn load(&self) -> CacheStatus {
        let resolver = self.resolver();
        resolver.affected(&[PathBuf::from("b/src/lib.rs")]).unwrap();
        resolver.cache_status().unwrap().clone()
    }
}

#[test]
fn first_load_misses_and_second_hits_without_running_cargo_metadata() {
    let fx = Fixture::new();
    assert_eq!(fx.load(), CacheStatus::Miss);
    assert_eq!(fx.metadata_runs(), 1);
    assert_eq!(fx.load(), CacheStatus::Hit);
    assert_eq!(fx.metadata_runs(), 1, "a hit must not run cargo metadata");
}

#[test]
fn a_hit_gives_the_same_answer_as_a_fresh_run() {
    let fx = Fixture::new();
    let changed = [PathBuf::from("b/src/lib.rs")];
    let fresh = fx.resolver().affected(&changed).unwrap();
    let cached = fx.resolver().affected(&changed).unwrap();
    assert_eq!(fresh, cached);
    assert_eq!(
        cached
            .packages
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[test]
fn one_resolver_runs_cargo_metadata_once_for_many_calls() {
    let fx = Fixture::new();
    let resolver = fx.resolver();
    for path in ["a/src/lib.rs", "b/src/lib.rs", "c/src/lib.rs"] {
        resolver.affected(&[PathBuf::from(path)]).unwrap();
    }
    assert_eq!(fx.metadata_runs(), 1);
}

#[test]
fn resolver_without_a_cache_dir_is_disabled_and_runs_metadata_each_time() {
    let fx = Fixture::new();
    for _ in 0..2 {
        let resolver = Resolver::new(&fx.root()).cargo(&fx.shim);
        resolver.affected(&[PathBuf::from("b/src/lib.rs")]).unwrap();
        assert_eq!(resolver.cache_status(), Some(&CacheStatus::Disabled));
    }
    assert_eq!(fx.metadata_runs(), 2);
}

#[test]
fn cargo_is_not_run_when_nothing_needs_the_graph() {
    let fx = Fixture::new();
    let resolver = fx.resolver();
    // Empty change set, a manifest, a lock file, a build script and an unmappable path.
    for changed in [
        vec![],
        vec!["a/Cargo.toml"],
        vec!["Cargo.lock"],
        vec!["b/build.rs"],
        vec!["../outside.rs"],
    ] {
        let changed: Vec<PathBuf> = changed.into_iter().map(PathBuf::from).collect();
        resolver.affected(&changed).unwrap();
    }
    assert_eq!(fx.metadata_runs(), 0);
    assert!(
        !fx.log.exists(),
        "cargo was not run at all, not even --version"
    );
    assert_eq!(resolver.cache_status(), None);
}

#[test]
fn editing_cargo_lock_invalidates() {
    let fx = Fixture::new();
    fx.load();
    let lock = fx.root().join("Cargo.lock");
    fs::write(
        &lock,
        format!("{}# edit\n", fs::read_to_string(&lock).unwrap()),
    )
    .unwrap();
    assert_eq!(fx.load(), CacheStatus::Miss);
    // Cargo normalises the edited lock file while it runs, so the key changes once more;
    // after that it is stable.
    fx.load();
    assert_eq!(fx.load(), CacheStatus::Hit);
}

#[test]
fn editing_a_member_manifest_invalidates() {
    let fx = Fixture::new();
    fx.load();
    write(
        &fx.root(),
        "c/Cargo.toml",
        &format!("{}# edit\n", manifest("c", "")),
    );
    assert_eq!(fx.load(), CacheStatus::Miss);
}

#[test]
fn a_new_member_picked_up_by_a_glob_invalidates() {
    let fx = Fixture::new();
    write(
        &fx.root(),
        "Cargo.toml",
        "[workspace]\nmembers = [\"*\"]\nresolver = \"2\"\n",
    );
    let before = fx.load();
    assert_eq!(before, CacheStatus::Miss);
    assert_eq!(fx.load(), CacheStatus::Hit);
    // The root manifest is untouched: only a new directory with a manifest appears.
    write(&fx.root(), "d/Cargo.toml", &manifest("d", ""));
    write(&fx.root(), "d/src/lib.rs", "");
    assert_eq!(fx.load(), CacheStatus::Miss);
    let resolver = fx.resolver();
    let affected = resolver.affected(&[PathBuf::from("d/src/lib.rs")]).unwrap();
    assert!(affected.packages.contains("d"), "{affected:?}");
}

#[test]
fn editing_a_path_dependency_outside_the_root_invalidates() {
    let fx = Fixture::new();
    fx.load();
    let ext = fx.dir.path().join("ext");
    write(
        &ext,
        "Cargo.toml",
        &format!("{}# edit\n", manifest("ext", "")),
    );
    assert_eq!(fx.load(), CacheStatus::Miss);
    assert_eq!(fx.load(), CacheStatus::Hit);
}

#[test]
fn a_different_cargo_version_invalidates() {
    let fx = Fixture::new();
    fx.load();
    let before = fx.metadata_runs();
    // The shim answers `--version` with other text, as another toolchain would.
    let other = Command::new(&fx.shim)
        .arg("--version")
        .env("CARGO_SHIM_VERSION", "cargo 0.0.0")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&other.stdout).trim(), "cargo 0.0.0");
    let version_shim = fx.dir.path().join("cargo-other");
    fs::write(
        &version_shim,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo cargo 0.0.0; exit 0; fi\nexec '{}' \"$@\"\n",
            fx.shim.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&version_shim, fs::Permissions::from_mode(0o755)).unwrap();
    let resolver = Resolver::new(&fx.root())
        .cargo(&version_shim)
        .cache_dir(fx.cache.path());
    resolver.affected(&[PathBuf::from("b/src/lib.rs")]).unwrap();
    assert_eq!(resolver.cache_status(), Some(&CacheStatus::Miss));
    assert_eq!(fx.metadata_runs(), before + 1);
}

#[test]
fn a_corrupt_or_truncated_entry_is_a_miss_not_an_error() {
    let fx = Fixture::new();
    fx.load();
    for entry in fs::read_dir(fx.cache.path()).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, &text[..text.len() / 2]).unwrap();
    }
    assert_eq!(fx.load(), CacheStatus::Miss);
    assert_eq!(fx.load(), CacheStatus::Hit);
}

#[test]
fn garbage_in_the_cache_file_is_a_miss() {
    let fx = Fixture::new();
    fx.load();
    for entry in fs::read_dir(fx.cache.path()).unwrap() {
        fs::write(entry.unwrap().path(), "not an entry").unwrap();
    }
    assert_eq!(fx.load(), CacheStatus::Miss);
}

#[test]
fn a_workspace_without_a_lock_file_is_not_cached_until_cargo_writes_one() {
    let fx = Fixture::new();
    fs::remove_file(fx.root().join("Cargo.lock")).unwrap();
    assert_eq!(fx.load(), CacheStatus::Disabled);
    assert_eq!(fs::read_dir(fx.cache.path()).unwrap().count(), 0);
    // That run made cargo write the lock file, so the next one has something to key on.
    assert!(fx.root().join("Cargo.lock").exists());
    assert_eq!(fx.load(), CacheStatus::Miss);
}

#[test]
fn an_unwritable_cache_dir_still_answers_and_says_why() {
    let fx = Fixture::new();
    let blocker = fx.dir.path().join("a-file");
    fs::write(&blocker, "").unwrap();
    let resolver = Resolver::new(&fx.root())
        .cargo(&fx.shim)
        .cache_dir(blocker.join("cache"));
    let affected = resolver.affected(&[PathBuf::from("b/src/lib.rs")]).unwrap();
    assert_eq!(affected.packages.len(), 2);
    assert!(
        matches!(resolver.cache_status(), Some(CacheStatus::MissNotStored(_))),
        "{:?}",
        resolver.cache_status()
    );
}

#[test]
fn two_workspaces_do_not_share_an_entry() {
    let first = Fixture::new();
    let second = Fixture::new();
    first.load();
    // Same cache dir, different root: still a miss.
    let resolver = Resolver::new(&second.root())
        .cargo(&second.shim)
        .cache_dir(first.cache.path());
    resolver.affected(&[PathBuf::from("b/src/lib.rs")]).unwrap();
    assert_eq!(resolver.cache_status(), Some(&CacheStatus::Miss));
}

#[test]
fn a_failing_cargo_metadata_is_an_error_with_cargos_message() {
    let fx = Fixture::new();
    let failing = fx.dir.path().join("cargo-fail");
    fs::write(
        &failing,
        "#!/bin/sh\nif [ \"$1\" = metadata ]; then echo 'boom from cargo' >&2; exit 101; fi\necho cargo 1\n",
    )
    .unwrap();
    fs::set_permissions(&failing, fs::Permissions::from_mode(0o755)).unwrap();
    let resolver = Resolver::new(&fx.root())
        .cargo(&failing)
        .cache_dir(fx.cache.path());
    let err = resolver
        .affected(&[PathBuf::from("b/src/lib.rs")])
        .unwrap_err();
    assert!(matches!(err, Error::Cargo(_)), "{err:?}");
    assert!(err.to_string().contains("boom from cargo"), "{err}");
    assert_eq!(
        fs::read_dir(fx.cache.path()).unwrap().count(),
        0,
        "a failure is not cached"
    );
}

#[test]
fn a_missing_cargo_executable_is_an_error() {
    let fx = Fixture::new();
    let resolver = Resolver::new(&fx.root()).cargo("/nonexistent/cargo");
    let err = resolver
        .affected(&[PathBuf::from("b/src/lib.rs")])
        .unwrap_err();
    assert!(matches!(err, Error::Cargo(_)), "{err:?}");
}
