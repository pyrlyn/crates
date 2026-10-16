//! Whole trees: how much a removal frees, copying one, and removing whatever is at a path.

use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::time::Instant;

/// What one walk found. A folder it could not read or finish is a lower bound, never an error.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub bytes: u64,
    /// Everything that is not a directory, a symlink included: the count a removal frees.
    pub files: u64,
    pub denied: bool,
    pub timed_out: bool,
}

/// Disk usage, not apparent size: allocated blocks on Unix so sparse files and APFS clones are
/// not over-counted, a hard link once. Elsewhere it is the apparent size with every link
/// counted, because std has no stable file id to dedupe by there. A symlink costs itself and is
/// never followed, so a link out of the tree cannot pull foreign bytes into the total.
pub fn usage(path: &Path) -> Usage {
    usage_until(path, None)
}

/// [`usage`] that stops at `deadline` (a huge cache on a network volume must not hang a
/// listing) and reports a directory it was not allowed to read.
pub fn usage_until(path: &Path, deadline: Option<Instant>) -> Usage {
    let mut usage = Usage::default();
    walk(path, &mut HashSet::new(), deadline, &mut usage);
    usage
}

fn walk(path: &Path, seen: &mut HashSet<(u64, u64)>, deadline: Option<Instant>, u: &mut Usage) {
    if u.timed_out || deadline.is_some_and(|d| Instant::now() >= d) {
        u.timed_out = true;
        return;
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            u.denied |= e.kind() == ErrorKind::PermissionDenied;
            return;
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() < 2 || seen.insert((meta.dev(), meta.ino())) {
            u.bytes += meta.blocks() * 512;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = &seen;
        u.bytes += meta.len();
    }
    u.files += u64::from(!meta.is_dir());
    if meta.is_dir() {
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    walk(&entry.path(), seen, deadline, u);
                }
            }
            Err(e) => u.denied |= e.kind() == ErrorKind::PermissionDenied,
        }
    }
}

/// What [`copy_tree`] does with a symlink inside the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Links {
    /// Copy what the link points at, so the copy stands alone (a plugin shipped as a tree).
    Follow,
    /// Recreate the link as it is (an app bundle whose links are part of it). Where links
    /// cannot be made (Windows without Developer Mode) the target is copied instead.
    Keep,
    /// Leave the link out: an unpacked archive whose links could point out of the tree.
    Skip,
}

/// What [`copy_tree_with`] calls with each copied file's path relative to the source and its
/// bytes; `Some` replaces the copy's bytes, `None` leaves the copy as it is.
pub type Fix<'a> = &'a mut dyn FnMut(&Path, &[u8]) -> Option<Vec<u8>>;

/// Copy the directory `src` into `dest`, creating `dest`. A regular file keeps its mode, and
/// no file is read back: a large bundle costs one copy, not a copy plus a read.
pub fn copy_tree(src: &Path, dest: &Path, links: Links) -> io::Result<()> {
    copy_dir(src, dest, links, Path::new(""), &mut None)
}

/// [`copy_tree`] with every copied file passed through `fix`.
pub fn copy_tree_with(src: &Path, dest: &Path, links: Links, fix: Fix) -> io::Result<()> {
    copy_dir(src, dest, links, Path::new(""), &mut Some(fix))
}

fn copy_dir(
    src: &Path,
    dest: &Path,
    links: Links,
    rel: &Path,
    fix: &mut Option<Fix>,
) -> io::Result<()> {
    fs::create_dir_all(dest).map_err(|e| crate::at(dest, e))?;
    for entry in fs::read_dir(src).map_err(|e| crate::at(src, e))? {
        let entry = entry.map_err(|e| crate::at(src, e))?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let rel = rel.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| crate::at(&from, e))?;
        if kind.is_symlink()
            && (links == Links::Skip || (links == Links::Keep && keep_link(&from, &to)?))
        {
            continue;
        }
        // A followed link to a directory is a directory to copy, not a file.
        if kind.is_dir() || (kind.is_symlink() && from.is_dir()) {
            copy_dir(&from, &to, links, &rel, fix)?;
            continue;
        }
        fs::copy(&from, &to).map_err(|e| crate::at(&from, e))?;
        if let Some(fix) = fix {
            let bytes = fs::read(&to).map_err(|e| crate::at(&to, e))?;
            if let Some(fixed) = fix(&rel, &bytes) {
                fs::write(&to, fixed).map_err(|e| crate::at(&to, e))?;
            }
        }
    }
    Ok(())
}

/// Recreate the link `from` at `to`; `false` when this platform cannot, so the caller copies.
#[cfg(unix)]
fn keep_link(from: &Path, to: &Path) -> io::Result<bool> {
    let target = fs::read_link(from).map_err(|e| crate::at(from, e))?;
    std::os::unix::fs::symlink(&target, to).map_err(|e| crate::at(to, e))?;
    Ok(true)
}

#[cfg(windows)]
fn keep_link(from: &Path, to: &Path) -> io::Result<bool> {
    let target = fs::read_link(from).map_err(|e| crate::at(from, e))?;
    let made = if from.is_dir() {
        std::os::windows::fs::symlink_dir(&target, to)
    } else {
        std::os::windows::fs::symlink_file(&target, to)
    };
    Ok(made.is_ok())
}

#[cfg(not(any(unix, windows)))]
fn keep_link(_: &Path, _: &Path) -> io::Result<bool> {
    Ok(false)
}

/// Remove whatever is at `path` (file, symlink or directory tree) without following a link.
/// Nothing there is success, so a retried cleanup does not fail on what the first one did.
pub fn remove_any(path: &Path) -> io::Result<()> {
    let result = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        // A Windows directory link reports as a symlink and is removed as a directory.
        Ok(meta) if cfg!(windows) && meta.file_type().is_symlink() && path.is_dir() => {
            fs::remove_dir(path)
        }
        Ok(_) => fs::remove_file(path),
        Err(e) => Err(e),
    };
    match result {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        other => other.map_err(|e| crate::at(path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fs-kit-tree-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn usage_counts_files_not_directories_and_stops_at_the_deadline() {
        let dir = tmp("usage");
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/x"), "x").unwrap();
        fs::write(dir.join("a/b/y"), vec![0u8; 5000]).unwrap();
        let u = usage(&dir);
        assert_eq!(u.files, 2);
        assert!(u.bytes >= 5000, "{u:?}");
        assert!(!u.timed_out && !u.denied);
        assert!(usage_until(&dir, Some(Instant::now())).timed_out);
        assert_eq!(usage(&dir.join("missing")), Usage::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_tree_copies_nested_files_through_fix() {
        let dir = tmp("copy");
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        fs::write(src.join("sub/b.txt"), "b").unwrap();
        let mut seen = Vec::new();
        copy_tree_with(&src, &dir.join("dest"), Links::Follow, &mut |rel, _| {
            seen.push(rel.to_path_buf());
            (rel == Path::new("sub/b.txt")).then(|| b"B".to_vec())
        })
        .unwrap();
        assert_eq!(fs::read(dir.join("dest/a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(dir.join("dest/sub/b.txt")).unwrap(), b"B");
        seen.sort();
        assert_eq!(seen, [PathBuf::from("a.txt"), PathBuf::from("sub/b.txt")]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn links_are_kept_or_followed_as_asked() {
        let dir = tmp("links");
        let src = dir.join("src");
        fs::create_dir_all(src.join("real")).unwrap();
        fs::write(src.join("real/f"), "f").unwrap();
        std::os::unix::fs::symlink("real", src.join("link")).unwrap();
        copy_tree(&src, &dir.join("kept"), Links::Keep).unwrap();
        let kept = fs::symlink_metadata(dir.join("kept/link")).unwrap();
        assert!(kept.file_type().is_symlink());
        copy_tree(&src, &dir.join("followed"), Links::Follow).unwrap();
        let followed = fs::symlink_metadata(dir.join("followed/link")).unwrap();
        assert!(followed.is_dir());
        assert_eq!(fs::read(dir.join("followed/link/f")).unwrap(), b"f");
        copy_tree(&src, &dir.join("skipped"), Links::Skip).unwrap();
        assert!(fs::symlink_metadata(dir.join("skipped/link")).is_err());
        assert_eq!(fs::read(dir.join("skipped/real/f")).unwrap(), b"f");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_any_takes_a_file_a_tree_and_nothing() {
        let dir = tmp("remove");
        fs::write(dir.join("f"), "f").unwrap();
        fs::create_dir_all(dir.join("d/e")).unwrap();
        remove_any(&dir.join("f")).unwrap();
        remove_any(&dir.join("d")).unwrap();
        remove_any(&dir.join("never")).unwrap();
        assert!(!dir.join("f").exists() && !dir.join("d").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remove_any_never_follows_a_link_into_its_target() {
        let dir = tmp("remove-link");
        fs::create_dir_all(dir.join("keep")).unwrap();
        fs::write(dir.join("keep/f"), "f").unwrap();
        std::os::unix::fs::symlink(dir.join("keep"), dir.join("link")).unwrap();
        remove_any(&dir.join("link")).unwrap();
        assert!(dir.join("keep/f").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
