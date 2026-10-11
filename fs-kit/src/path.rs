//! Path identity: Windows case, and the `\\?\` prefix `std::fs::canonicalize` adds. One copy,
//! so a root one program drops is the same root another refuses to walk.

use std::path::{Component, Path, PathBuf};

/// Existing path, without the `\\?\` prefix `std::fs::canonicalize` adds on Windows.
/// A missing path is returned with that prefix stripped lexically.
pub fn canon(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| simplified(path).to_path_buf())
}

/// [`std::fs::canonicalize`] without the Windows `\\?\` prefix; an error for a missing path.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// `path` without a `\\?\` prefix where dropping it keeps the meaning; no disk access.
pub fn simplified(path: &Path) -> &Path {
    dunce::simplified(path)
}

/// Whether `root` is the filesystem root or `home`, which no index or walk may use as a
/// project root. `home` is a parameter so tests need not touch `$HOME`.
pub fn is_unwalkable_root(root: &Path, home: Option<&Path>) -> bool {
    let root = canon(root);
    let is_home = home.is_some_and(|h| same_path(&canon(h), &root));
    root.parent().is_none() || is_home
}

/// Whether two paths name one existing entry (`/var` vs `/private/var`, `RUNNER~1`, `\\?\`).
/// A path that does not resolve matches only its own spelling.
pub fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || canonicalize(a)
            .ok()
            .is_some_and(|a| canonicalize(b).ok() == Some(a))
}

/// `path` is `root` or a file under it. On Windows the check is ASCII-case-insensitive
/// and a `\\?\` prefix does not count as a different directory.
pub fn path_starts_with(path: &Path, root: &Path) -> bool {
    starts_with(path, root, cfg!(windows))
}

fn starts_with(path: &Path, root: &Path, fold: bool) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }
    let path = simplified(path);
    let root = simplified(root);
    if path.starts_with(root) {
        return true;
    }
    fold && components_prefix_ci(path, root)
}

/// The same path, including Windows case and a `\\?\` prefix; no disk access.
pub fn same_path(a: &Path, b: &Path) -> bool {
    same(a, b, cfg!(windows))
}

fn same(a: &Path, b: &Path, fold: bool) -> bool {
    let a = simplified(a);
    let b = simplified(b);
    if a == b {
        return true;
    }
    fold && components_eq_ci(a, b)
}

/// `suffix` names the trailing components of `path` (`src/a.rs` in `/repo/src/a.rs`), whole
/// components only. On Windows the comparison is ASCII-case-insensitive.
pub fn path_ends_with(path: &Path, suffix: &Path) -> bool {
    ends_with(path, suffix, cfg!(windows))
}

fn ends_with(path: &Path, suffix: &Path, fold: bool) -> bool {
    // `Path::ends_with("")` is true; an empty suffix names nothing.
    if suffix.as_os_str().is_empty() {
        return false;
    }
    if path.ends_with(suffix) {
        return true;
    }
    if !fold {
        return false;
    }
    let path_c: Vec<_> = path.components().collect();
    let suf_c: Vec<_> = suffix.components().collect();
    suf_c.len() <= path_c.len()
        && path_c[path_c.len() - suf_c.len()..]
            .iter()
            .zip(&suf_c)
            .all(|(p, s)| components_match(*p, *s))
}

/// `path` with `prefix` removed. On Windows, ASCII case and a `\\?\` prefix still match.
pub fn strip_prefix(path: &Path, prefix: &Path) -> Option<PathBuf> {
    strip(path, prefix, cfg!(windows))
}

fn strip(path: &Path, prefix: &Path, fold: bool) -> Option<PathBuf> {
    let path = simplified(path);
    let prefix = simplified(prefix);
    if let Ok(rel) = path.strip_prefix(prefix) {
        return Some(rel.to_path_buf());
    }
    if !fold || !components_prefix_ci(path, prefix) {
        return None;
    }
    let skip = prefix.components().count();
    Some(path.components().skip(skip).collect())
}

/// Lexical join of `path` onto `root` (`..` pops, `.` drops): no disk access, so a missing
/// path still normalises. An absolute `path` replaces `root`.
///
/// Deliberately not clamped to `root`: a linked worktree's `gitdir: ../../main/.git/…`
/// legitimately climbs above it. Confinement is the caller's check, after the join.
pub fn normalize(root: &Path, path: &Path) -> PathBuf {
    let mut out = if path.is_absolute() {
        PathBuf::new()
    } else {
        root.to_path_buf()
    };
    for c in path.components() {
        match c {
            // Push, do not replace: on Windows `C:\foo` is Prefix("C:") then RootDir, and
            // replacing wiped the drive.
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::Prefix(p) => out = PathBuf::from(p.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(s) => out.push(s),
        }
    }
    out
}

fn components_match(a: Component, b: Component) -> bool {
    match (a, b) {
        (Component::Normal(x), Component::Normal(y)) => x.eq_ignore_ascii_case(y),
        (Component::Prefix(x), Component::Prefix(y)) => {
            x.as_os_str().eq_ignore_ascii_case(y.as_os_str())
        }
        (x, y) => x == y,
    }
}

fn components_prefix_ci(path: &Path, root: &Path) -> bool {
    let path_c: Vec<_> = path.components().collect();
    let root_c: Vec<_> = root.components().collect();
    !root_c.is_empty()
        && root_c.len() <= path_c.len()
        && path_c
            .iter()
            .zip(&root_c)
            .all(|(p, r)| components_match(*p, *r))
}

fn components_eq_ci(a: &Path, b: &Path) -> bool {
    let a: Vec<_> = a.components().collect();
    let b: Vec<_> = b.components().collect();
    a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| components_match(*x, *y))
}

#[cfg(test)]
mod tests {
    use super::*;

    // `fold` is a parameter so the Windows branch runs on every CI platform.
    #[test]
    fn suffix_match_folds_ascii_case_only_when_asked() {
        let abs = Path::new("/Repo/Src/Main.rs");
        let rel = Path::new("src/main.rs");
        assert!(ends_with(abs, rel, true));
        assert!(!ends_with(abs, rel, false));
        assert!(ends_with(abs, Path::new("Src/Main.rs"), false));
        // Whole components: a partial file name never matches, folded or not.
        assert!(!ends_with(abs, Path::new("ain.rs"), true));
        assert!(!ends_with(abs, Path::new(""), true));
        assert!(!ends_with(rel, abs, true));
    }

    #[test]
    fn prefix_and_equality_fold_case_only_when_asked() {
        let path = Path::new("/Repo/Src/main.rs");
        assert!(starts_with(path, Path::new("/repo/src"), true));
        assert!(!starts_with(path, Path::new("/repo/src"), false));
        assert!(!starts_with(path, Path::new(""), true));
        assert!(!starts_with(
            Path::new("/repo"),
            Path::new("/repo/src"),
            true
        ));
        assert!(same(Path::new("/A/b"), Path::new("/a/B"), true));
        assert!(!same(Path::new("/A/b"), Path::new("/a/B"), false));
        assert_eq!(
            strip(path, Path::new("/repo"), true),
            Some(PathBuf::from("Src/main.rs"))
        );
        assert_eq!(strip(path, Path::new("/repo"), false), None);
        assert_eq!(strip(path, Path::new("/other"), true), None);
    }

    #[test]
    fn normalize_pops_and_drops_without_touching_disk() {
        let root = Path::new("/w/repo");
        assert_eq!(
            normalize(root, Path::new("src/./a.rs")),
            Path::new("/w/repo/src/a.rs")
        );
        assert_eq!(
            normalize(root, Path::new("../../main/.git")),
            Path::new("/main/.git")
        );
        assert_eq!(normalize(root, Path::new("/etc/x")), Path::new("/etc/x"));
    }

    #[test]
    fn canon_resolves_links_and_keeps_a_missing_path() {
        let dir = std::env::temp_dir().join(format!("fs-kit-canon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = canonicalize(&dir).unwrap();
        assert_eq!(canon(&dir.join(".")), real);
        let missing = dir.join("nope");
        assert_eq!(canon(&missing), missing);
        assert!(same_file(&dir, &real));
        assert!(!same_file(&missing, &dir));
        assert!(is_unwalkable_root(Path::new("/"), None));
        assert!(is_unwalkable_root(&dir, Some(&real)));
        assert!(!is_unwalkable_root(&dir, None));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
