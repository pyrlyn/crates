// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The on-disk layout of a user plugin and the only code that changes it:
//! staging a package into `versions/<digest12>/`, moving the `current` /
//! `previous` pointers, pointing `link` at a development directory, and
//! deleting the whole `<id>` directory. Discovery only reads this layout.
//!
//! ```text
//! <home>/plugins/<id>/
//!   current            digest12 of the active version
//!   previous           digest12 of the one kept for rollback
//!   link               absolute path of a development package, if any
//!   versions/<digest12>/
//! ```
//!
//! Copying a tree, fsyncing it and removing a directory are local until
//! roadmap T16 `dir-ops` exists (`copy_tree`, directory fsync, `remove_any`).
//! That crate draws on this file; call sites stay here so the swap is one
//! edit.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use crate::digest::package_digest;

/// The pointer file naming the active version.
pub const CURRENT: &str = "current";
/// The pointer file naming the one version kept for rollback.
pub const PREVIOUS: &str = "previous";
/// The pointer file naming a development package read in place.
pub const LINK: &str = "link";

/// `<home>/plugins/<id>`. `id` must be a single path segment that does not
/// start with `.`, so it cannot name a path outside that root.
pub fn plugin_dir(home: &Path, id: &str) -> io::Result<PathBuf> {
    if !id_is_safe(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("plugin id {id:?} is not a single path segment"),
        ));
    }
    Ok(home.join("plugins").join(id))
}

/// The directory name of a version: the first 12 hex digits of its digest.
pub fn short(digest: &str) -> &str {
    digest.get(..12).unwrap_or(digest)
}

/// A version directory name: non-empty hex, at most the 64 digits of SHA-256.
pub(crate) fn is_version_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Copies `src` into `<plugin_dir>/versions/<digest12>/` through a
/// `<digest12>.tmp` sibling that is fsynced and renamed into place, so a
/// crash never leaves a half-written version under its final name. The copy
/// is digested again before the rename: a grant is decided against `digest`,
/// so bytes that changed after validation must never land under it. An
/// existing version directory is reused only if it still digests to `digest`.
///
/// `digest` is the full lowercase hex string [`package_digest`](crate::package_digest)
/// returns, not the 12-digit prefix.
pub fn stage(src: &Path, plugin_dir: &Path, digest: &str) -> io::Result<PathBuf> {
    if !is_version_id(digest) || digest.len() < 12 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "digest must be at least 12 hex digits",
        ));
    }
    let versions = plugin_dir.join("versions");
    fs::create_dir_all(&versions)?;
    let digest12 = short(digest);
    let dest = versions.join(digest12);
    if let Some(reused) = take_existing(&dest, digest)? {
        return Ok(reused);
    }
    let tmp = versions.join(format!("{digest12}.tmp"));
    // A leftover temp is never a finished version. Drop the symlink itself
    // when one was planted; do not follow it.
    if fs::symlink_metadata(&tmp).is_ok() {
        remove_path(&tmp)?;
    }
    copy_synced(src, &tmp)?;
    if package_digest(&tmp)? != digest {
        fs::remove_dir_all(&tmp)?;
        return Err(io::Error::other(
            "the package changed while it was being copied; run the command again",
        ));
    }
    fs::rename(&tmp, &dest)?;
    sync_dir(&versions);
    Ok(dest)
}

/// Deletes a user plugin's whole directory. Canonicalizes the target first
/// and refuses to touch it unless it resolves to somewhere inside
/// `<home>/plugins/` — a plugin directory that has become a symlink pointing
/// outside that root is left alone. Once past that check, `remove_dir_all`
/// does not follow a symlink it finds inside, it only unlinks it, so an
/// inner symlink loses only itself. A directory that is already gone is not
/// an error, so remove is safe to retry.
pub fn remove(home: &Path, id: &str) -> io::Result<()> {
    let dir = plugin_dir(home, id)?;
    if !dir.exists() {
        return Ok(());
    }
    let plugins_root = fs::canonicalize(home.join("plugins"))?;
    let resolved = fs::canonicalize(&dir)?;
    if !resolved.starts_with(&plugins_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to remove {}: resolves to {}, outside {}",
                dir.display(),
                resolved.display(),
                plugins_root.display()
            ),
        ));
    }
    fs::remove_dir_all(&resolved)
}

/// Reads a pointer file (`CURRENT`, `PREVIOUS` or `LINK`). `None` when it
/// is missing or empty.
pub fn read_pointer(plugin_dir: &Path, name: &str) -> io::Result<Option<String>> {
    if !pointer_name_is_safe(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("pointer name {name:?} is not current, previous or link"),
        ));
    }
    match fs::read_to_string(plugin_dir.join(name)) {
        Ok(text) => {
            let text = text.trim();
            Ok((!text.is_empty()).then(|| text.to_string()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Makes the staged `digest12` current. The version it replaces becomes
/// `previous`, and every other version is deleted, so at most two stay on
/// disk. Rolling back is this same call with the previous digest: the two
/// pointers trade places. The caller decides whether that version may run;
/// this function never asks.
pub fn activate(plugin_dir: &Path, digest12: &str) -> io::Result<()> {
    if !is_version_id(digest12) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "version id must be hex",
        ));
    }
    let old = read_pointer(plugin_dir, CURRENT)?;
    if old.as_deref() != Some(digest12) {
        if let Some(old) = &old {
            write_pointer(plugin_dir, PREVIOUS, old)?;
        }
        write_pointer(plugin_dir, CURRENT, digest12)?;
    }
    let previous = read_pointer(plugin_dir, PREVIOUS)?;
    prune(plugin_dir, digest12, previous.as_deref())
}

/// Points `<plugin_dir>/link` at `src`. Discovery prefers this over
/// `current` when both are present, and reads `src` in place — nothing is
/// copied. Calling this again with a different `src` repoints it. Removing
/// the plugin, or deleting the file, falls back to `current`.
///
/// `src` is stored as text, so it must be valid Unicode and must not contain
/// a newline. The application trusts the path: a link is written by the
/// user, not by a package.
pub fn link(plugin_dir: &Path, src: &Path) -> io::Result<()> {
    let text = src.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "link target path is not valid Unicode",
        )
    })?;
    if text.contains(['\n', '\r']) || text.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "link target path must be a single non-empty line",
        ));
    }
    write_pointer(plugin_dir, LINK, text)
}

/// `Ok(Some(dest))` when `dest` is a real directory that still digests to
/// `digest`. A symlink is unlinked and not followed. Anything else is
/// removed so the caller can write a new directory.
fn take_existing(dest: &Path, digest: &str) -> io::Result<Option<PathBuf>> {
    let meta = match fs::symlink_metadata(dest) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() {
        fs::remove_file(dest)?;
        return Ok(None);
    }
    if meta.is_dir() {
        if package_digest(dest)? == digest {
            return Ok(Some(dest.to_path_buf()));
        }
        fs::remove_dir_all(dest)?;
        return Ok(None);
    }
    fs::remove_file(dest)?;
    Ok(None)
}

fn remove_path(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn id_is_safe(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && !id.contains(['/', '\\', '\0'])
        && id != "."
        && id != ".."
}

fn pointer_name_is_safe(name: &str) -> bool {
    name == CURRENT || name == PREVIOUS || name == LINK
}

/// Writes a pointer via a temp file plus rename, atomic on one filesystem,
/// so a reader sees the old name or the new one, never a torn write.
fn write_pointer(plugin_dir: &Path, name: &str, value: &str) -> io::Result<()> {
    fs::create_dir_all(plugin_dir)?;
    let tmp = plugin_dir.join(format!("{name}.tmp"));
    fs::write(&tmp, value)?;
    File::open(&tmp)?.sync_all()?;
    fs::rename(&tmp, plugin_dir.join(name))?;
    sync_dir(plugin_dir);
    Ok(())
}

/// Deletes every entry under `versions/` except `current` and `previous`,
/// including an unapproved staged version or a leftover `.tmp`.
fn prune(plugin_dir: &Path, current: &str, previous: Option<&str>) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(plugin_dir.join("versions")) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if name == current || previous.is_some_and(|p| name == p) {
            continue;
        }
        // `file_type` does not follow symlinks, and neither does
        // `remove_dir_all`, so a link out of `versions/` loses only itself.
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Copies regular files and directories, fsyncing each file. Symlinks are
/// skipped, the same files `package_digest` skips, so the copy digests the
/// same as the source.
fn copy_synced(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_synced(&entry.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), &to)?;
            File::open(&to)?.sync_all()?;
        }
    }
    sync_dir(dst);
    Ok(())
}

/// Best effort: a directory fsync makes a rename durable on Unix; other
/// platforms cannot open a directory as a file, and there the rename alone
/// is what the OS offers.
fn sync_dir(dir: &Path) {
    if let Ok(f) = File::open(dir) {
        let _ = f.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(dir: &Path, wasm: &str) -> String {
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("plugin.toml"), "id = \"demo\"\n").unwrap();
        fs::write(dir.join("bin/plugin.wasm"), wasm).unwrap();
        package_digest(dir).unwrap()
    }

    #[test]
    fn staged_version_digests_the_same_and_leaves_no_tmp() {
        let src = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let digest = package(src.path(), "one");
        let plugin = plugin_dir(home.path(), "demo").unwrap();

        let dest = stage(src.path(), &plugin, &digest).unwrap();

        assert_eq!(dest, plugin.join("versions").join(short(&digest)));
        assert_eq!(package_digest(&dest).unwrap(), digest);
        assert!(
            !plugin
                .join(format!("versions/{}.tmp", short(&digest)))
                .exists()
        );
    }

    #[test]
    fn stage_refuses_bytes_that_differ_from_the_validated_digest() {
        let src = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let validated = package(src.path(), "one");
        package(src.path(), "two");
        let plugin = plugin_dir(home.path(), "demo").unwrap();

        assert!(stage(src.path(), &plugin, &validated).is_err());
        let versions: Vec<_> = fs::read_dir(plugin.join("versions")).unwrap().collect();
        assert!(
            versions.is_empty(),
            "nothing lands under the granted digest"
        );
    }

    #[test]
    #[cfg(unix)]
    fn stage_replaces_a_planted_dest_symlink_and_leaves_the_target() {
        let src = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("canary"), b"keep").unwrap();
        let digest = package(src.path(), "one");
        let plugin = plugin_dir(home.path(), "demo").unwrap();
        let versions = plugin.join("versions");
        fs::create_dir_all(&versions).unwrap();
        std::os::unix::fs::symlink(outside.path(), versions.join(short(&digest))).unwrap();

        let dest = stage(src.path(), &plugin, &digest).unwrap();

        assert!(dest.join("bin/plugin.wasm").is_file());
        assert!(outside.path().join("canary").is_file());
        assert!(!dest.is_symlink());
    }

    #[test]
    fn stage_refuses_a_digest_that_is_not_hex() {
        let src = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        package(src.path(), "one");
        let plugin = plugin_dir(home.path(), "demo").unwrap();

        assert!(stage(src.path(), &plugin, "../not-a-digest").is_err());
        assert!(!plugin.join("versions").exists());
    }

    #[test]
    fn link_writes_a_readable_pointer_and_repointing_overwrites_it() {
        let home = tempfile::tempdir().unwrap();
        let plugin = plugin_dir(home.path(), "demo").unwrap();
        let src_a = tempfile::tempdir().unwrap();
        let src_b = tempfile::tempdir().unwrap();

        link(&plugin, src_a.path()).unwrap();
        assert_eq!(
            read_pointer(&plugin, LINK).unwrap().as_deref(),
            Some(src_a.path().to_str().unwrap())
        );

        link(&plugin, src_b.path()).unwrap();
        assert_eq!(
            read_pointer(&plugin, LINK).unwrap().as_deref(),
            Some(src_b.path().to_str().unwrap())
        );
    }

    #[test]
    fn activate_keeps_one_previous_and_rollback_swaps_the_pointers() {
        let home = tempfile::tempdir().unwrap();
        let plugin = plugin_dir(home.path(), "demo").unwrap();
        let mut digests = Vec::new();
        for wasm in ["one", "two", "three"] {
            let src = tempfile::tempdir().unwrap();
            let digest = package(src.path(), wasm);
            stage(src.path(), &plugin, &digest).unwrap();
            activate(&plugin, short(&digest)).unwrap();
            digests.push(short(&digest).to_string());
        }
        let current = read_pointer(&plugin, CURRENT).unwrap();
        let previous = read_pointer(&plugin, PREVIOUS).unwrap();
        assert_eq!(current.as_deref(), Some(digests[2].as_str()));
        assert_eq!(previous.as_deref(), Some(digests[1].as_str()));
        assert!(!plugin.join("versions").join(&digests[0]).exists());

        activate(&plugin, &digests[1]).unwrap();

        assert_eq!(
            read_pointer(&plugin, CURRENT).unwrap().as_deref(),
            Some(digests[1].as_str())
        );
        assert_eq!(
            read_pointer(&plugin, PREVIOUS).unwrap().as_deref(),
            Some(digests[2].as_str())
        );
        assert!(plugin.join("versions").join(&digests[2]).is_dir());
    }

    #[test]
    fn remove_deletes_the_plugin_directory() {
        let home = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let digest = package(src.path(), "one");
        let plugin = plugin_dir(home.path(), "demo").unwrap();
        stage(src.path(), &plugin, &digest).unwrap();
        activate(&plugin, short(&digest)).unwrap();
        assert!(plugin.exists());

        remove(home.path(), "demo").unwrap();

        assert!(!plugin.exists());
    }

    #[test]
    fn remove_also_clears_the_link_pointer() {
        let home = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let plugin = plugin_dir(home.path(), "demo").unwrap();
        link(&plugin, src.path()).unwrap();
        assert!(plugin.join(LINK).exists());

        remove(home.path(), "demo").unwrap();

        assert!(!plugin.join(LINK).exists());
        assert!(!plugin.exists());
    }

    #[test]
    fn remove_of_a_missing_plugin_is_a_no_op() {
        let home = tempfile::tempdir().unwrap();
        assert!(remove(home.path(), "ghost").is_ok());
    }

    #[test]
    fn remove_refuses_an_id_that_escapes_the_plugins_root() {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join("canary"), b"keep").unwrap();

        assert!(remove(home.path(), "..").is_err());
        assert!(home.path().join("canary").is_file());
    }

    /// A plugin directory that has become a symlink pointing outside
    /// `<home>/plugins/` must never be followed: `remove` refuses it and
    /// the escaped target is untouched.
    #[test]
    #[cfg(unix)]
    fn remove_refuses_path_outside_plugins_dir() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("canary"), b"keep").unwrap();
        // Canonicalize before comparing: on macOS the tempdir base itself
        // sits behind a symlink (`/tmp` -> `/private/tmp`).
        let outside_root = fs::canonicalize(outside.path()).unwrap();
        fs::create_dir_all(home.path().join("plugins")).unwrap();
        std::os::unix::fs::symlink(&outside_root, home.path().join("plugins/evil")).unwrap();

        let result = remove(home.path(), "evil");

        assert!(
            result.is_err(),
            "must refuse a plugin dir that resolves outside plugins/"
        );
        assert!(
            outside_root.join("canary").exists(),
            "must not touch the escaped target"
        );
    }
}
