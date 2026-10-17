//! Each case builds a throwaway repository; the host git config must not leak in.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use git_changed_paths::{Error, changed_paths};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// `main` holds a.txt, b.txt, sub/c.txt and a .gitignore; the repo sits on `feature`.
fn repo() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(dir, "a.txt", "a\n");
    write(dir, "b.txt", "b\n");
    write(dir, "sub/c.txt", "c\n");
    write(dir, ".gitignore", "*.log\n");
    commit_all(dir, "initial");
    git(dir, &["checkout", "-q", "-b", "feature"]);
    tmp
}

fn paths(repo: &Path) -> BTreeSet<PathBuf> {
    changed_paths(repo, "main").unwrap().paths
}

fn set(items: &[&str]) -> BTreeSet<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

#[test]
fn clean_tree_has_no_changes() {
    let tmp = repo();
    let result = changed_paths(tmp.path(), "main").unwrap();
    assert!(result.paths.is_empty());
    assert_eq!(result.merge_base, git(tmp.path(), &["rev-parse", "main"]));
}

#[test]
fn committed_change() {
    let tmp = repo();
    write(tmp.path(), "a.txt", "changed\n");
    commit_all(tmp.path(), "edit a");
    assert_eq!(paths(tmp.path()), set(&["a.txt"]));
}

#[test]
fn commits_on_the_base_after_the_fork_are_excluded() {
    let tmp = repo();
    let dir = tmp.path();
    git(dir, &["checkout", "-q", "main"]);
    write(dir, "only-on-main.txt", "m\n");
    commit_all(dir, "main moves on");
    git(dir, &["checkout", "-q", "feature"]);
    write(dir, "b.txt", "changed\n");
    commit_all(dir, "edit b");
    assert_eq!(paths(dir), set(&["b.txt"]));
}

#[test]
fn staged_change() {
    let tmp = repo();
    write(tmp.path(), "a.txt", "staged\n");
    git(tmp.path(), &["add", "a.txt"]);
    assert_eq!(paths(tmp.path()), set(&["a.txt"]));
}

#[test]
fn unstaged_change() {
    let tmp = repo();
    write(tmp.path(), "b.txt", "unstaged\n");
    assert_eq!(paths(tmp.path()), set(&["b.txt"]));
}

#[test]
fn untracked_file() {
    let tmp = repo();
    write(tmp.path(), "new/dir/file.txt", "n\n");
    assert_eq!(paths(tmp.path()), set(&["new/dir/file.txt"]));
}

#[test]
fn ignored_file_is_excluded() {
    let tmp = repo();
    write(tmp.path(), "debug.log", "noise\n");
    write(tmp.path(), "kept.txt", "k\n");
    assert_eq!(paths(tmp.path()), set(&["kept.txt"]));
}

#[test]
fn deleted_file() {
    let tmp = repo();
    fs::remove_file(tmp.path().join("a.txt")).unwrap();
    assert_eq!(paths(tmp.path()), set(&["a.txt"]));
}

#[test]
fn staged_then_removed_file_is_not_missed() {
    let tmp = repo();
    write(tmp.path(), "tmp.txt", "t\n");
    git(tmp.path(), &["add", "tmp.txt"]);
    fs::remove_file(tmp.path().join("tmp.txt")).unwrap();
    assert_eq!(paths(tmp.path()), set(&["tmp.txt"]));
}

#[test]
fn committed_rename_reports_both_paths() {
    let tmp = repo();
    git(tmp.path(), &["mv", "a.txt", "renamed.txt"]);
    commit_all(tmp.path(), "rename");
    assert_eq!(paths(tmp.path()), set(&["a.txt", "renamed.txt"]));
}

#[test]
fn staged_rename_reports_both_paths() {
    let tmp = repo();
    git(tmp.path(), &["mv", "sub/c.txt", "moved.txt"]);
    assert_eq!(paths(tmp.path()), set(&["sub/c.txt", "moved.txt"]));
}

#[test]
fn unknown_base_is_an_error() {
    let tmp = repo();
    let err = changed_paths(tmp.path(), "no-such-ref").unwrap_err();
    assert!(
        matches!(err, Error::BaseUnknown(ref b) if b == "no-such-ref"),
        "{err:?}"
    );
}

#[test]
fn option_like_base_is_rejected() {
    let tmp = repo();
    let err = changed_paths(tmp.path(), "--all").unwrap_err();
    assert!(matches!(err, Error::BaseUnknown(_)), "{err:?}");
}

#[test]
fn directory_outside_a_repository_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let err = changed_paths(tmp.path(), "main").unwrap_err();
    assert!(matches!(err, Error::NotARepository { .. }), "{err:?}");
}

#[test]
fn unrelated_history_has_no_merge_base() {
    let tmp = repo();
    let dir = tmp.path();
    git(dir, &["checkout", "-q", "--orphan", "island"]);
    git(dir, &["rm", "-rfq", "."]);
    write(dir, "island.txt", "i\n");
    commit_all(dir, "island");
    let err = changed_paths(dir, "main").unwrap_err();
    assert!(matches!(err, Error::NoMergeBase(_)), "{err:?}");
}

#[test]
fn shallow_clone_without_a_merge_base_says_so() {
    let tmp = repo();
    let src = tmp.path();
    write(src, "f.txt", "f\n");
    commit_all(src, "feature work");
    git(src, &["checkout", "-q", "main"]);
    write(src, "m.txt", "m\n");
    commit_all(src, "main moves on");
    git(src, &["checkout", "-q", "feature"]);

    // Depth 1 keeps only the tips, so the fork point is cut off on both sides.
    let dst = tempfile::tempdir().unwrap();
    let url = format!("file://{}", src.display());
    git(
        dst.path(),
        &["clone", "-q", "--depth=1", "--no-single-branch", &url, "."],
    );
    git(dst.path(), &["checkout", "-q", "feature"]);
    let err = changed_paths(dst.path(), "origin/main").unwrap_err();
    assert!(matches!(err, Error::ShallowNoMergeBase(_)), "{err:?}");
    let text = err.to_string();
    assert!(
        text.contains("shallow") && text.contains("fetch-depth: 0") && text.contains("--unshallow"),
        "{text}"
    );

    // Deepening restores the fork point, and the query then succeeds.
    git(dst.path(), &["fetch", "-q", "--unshallow"]);
    let result = changed_paths(dst.path(), "origin/main").unwrap();
    assert_eq!(result.paths, set(&["f.txt"]));
}

#[test]
fn subdirectory_argument_yields_top_level_relative_paths() {
    let tmp = repo();
    write(tmp.path(), "sub/c.txt", "changed\n");
    write(tmp.path(), "a.txt", "changed\n");
    let result = changed_paths(&tmp.path().join("sub"), "main").unwrap();
    assert_eq!(result.paths, set(&["a.txt", "sub/c.txt"]));
}
