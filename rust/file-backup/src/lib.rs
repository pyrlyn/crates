// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Copy a file to `<name>.bak-<unix-seconds>` before replacing it.
//!
//! Shared by ketch (`config reset`) and rtok (agent host setup). Two targets:
//!
//! - [`backup`] puts the copy beside the file; a sibling `.bak-*` that already holds the
//!   same bytes makes it `Ok(None)`.
//! - [`Folder`] puts it in a subfolder next to the file (default [`DEFAULT_FOLDER`]),
//!   treats any regular file there with the same bytes as the backup, and prunes the
//!   folder down to the newest `keep` generations of that name.
//!
//! In both, a missing file is `Ok(None)` — a second reset or setup must not grow a stack
//! of identical undos.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use same_file::Handle;

/// Folder name next to a config file that holds undo copies, for [`Folder`].
pub const DEFAULT_FOLDER: &str = "_backup";

/// Copy `path` to `<name>.bak-<unix-seconds>` beside it.
///
/// `None` when there is no file yet, or when a `<name>.bak-*` sibling already
/// holds the same bytes.
pub fn backup(path: &Path) -> std::io::Result<Option<PathBuf>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    backup_at(path, now())
}

/// [`backup`] with the clock passed in, so a test can pin the second.
pub fn backup_at(path: &Path, ts: u64) -> std::io::Result<Option<PathBuf>> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new(""));
    copy_unless_present(path, ts, dir, &name, false)
}

/// Backups in a subfolder next to the file, capped at the newest `keep` generations.
///
/// Copies land in `<parent>/<name>/<file name>.bak-<ts>[-<n>]`. `keep == 0` keeps all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Folder<'a> {
    /// Folder name, one path component. Pruning only ever touches a folder of this name.
    pub name: &'a str,
    /// Newest generations of one file kept by [`Folder::prune`]; `0` keeps all.
    pub keep: usize,
}

impl Folder<'_> {
    /// Copy `path` into the folder and prune it. `None` when there is no file yet, or
    /// when any regular file in the folder already holds the same bytes, whatever its
    /// name — an unchanged second run is the same undo.
    pub fn backup(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        let bak = self.backup_at(path, now())?;
        if let Some(b) = &bak {
            self.prune(b);
        }
        Ok(bak)
    }

    /// [`Folder::backup`] with the clock passed in and without pruning.
    ///
    /// The suffix goes past the highest `-<n>` already used for `ts`, so the copy never
    /// reuses a slot pruning freed and always sorts newest.
    pub fn backup_at(&self, path: &Path, ts: u64) -> io::Result<Option<PathBuf>> {
        let mut parts = Path::new(self.name).components();
        // A name like `../x` would put copies outside the file's directory.
        if !matches!(
            (parts.next(), parts.next()),
            (Some(Component::Normal(_)), None)
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "backup folder name must be one plain path component",
            ));
        }
        let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(OsStr::to_str))
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "backup source needs a parent and a UTF-8 file name",
            ));
        };
        copy_unless_present(path, ts, &parent.join(self.name), name, true)
    }

    /// Delete the oldest `<name>.bak-<ts>[-<n>]` generations beside `kept` — a copy
    /// [`Folder::backup`] returned — until `keep` remain. `kept` itself is never deleted.
    /// Only regular files with that exact name shape inside a folder named
    /// [`Folder::name`] are candidates. Best effort: an fs error leaves the rest in
    /// place and is not reported.
    pub fn prune(&self, kept: &Path) {
        let (Some(dir), Some(file)) = (kept.parent(), kept.file_name().and_then(OsStr::to_str))
        else {
            return;
        };
        if self.keep == 0 || !self.is_folder(dir) {
            return;
        }
        let Some((name, _)) = file.rsplit_once(".bak-") else {
            return;
        };
        let mut older = generations(dir, name);
        older.retain(|(_, p)| p != kept);
        older.sort();
        let excess = (older.len() + 1).saturating_sub(self.keep);
        for (_, p) in older.into_iter().take(excess) {
            let _ = fs::remove_file(p);
        }
    }

    /// The generations in the folder `dir` that a cap of `keep` has no room for: per base
    /// name, all but the newest `keep`, oldest first. Read-only, the listing twin of
    /// [`Folder::prune`] for a cap lowered after the copies were taken; `0` keeps all.
    pub fn stale(&self, dir: &Path) -> Vec<PathBuf> {
        if self.keep == 0 || !self.is_folder(dir) {
            return Vec::new();
        }
        let mut names: Vec<String> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let f = e.ok()?.file_name().into_string().ok()?;
                Some(f.rsplit_once(".bak-")?.0.to_string())
            })
            .collect();
        names.sort();
        names.dedup();
        let mut out = Vec::new();
        for name in names {
            let mut all = generations(dir, &name);
            all.sort();
            let excess = all.len().saturating_sub(self.keep);
            out.extend(all.into_iter().take(excess).map(|(_, p)| p));
        }
        out
    }

    fn is_folder(&self, dir: &Path) -> bool {
        dir.file_name().is_some_and(|d| d == self.name)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The copy loop both targets share. `in_folder` widens the dedup check from `<name>.bak-*`
/// siblings to every file in `dir`, creates `dir`, and starts the suffix past the used ones.
fn copy_unless_present(
    path: &Path,
    ts: u64,
    dir: &Path,
    name: &str,
    in_folder: bool,
) -> io::Result<Option<PathBuf>> {
    fs::symlink_metadata(path)?;
    let body = fs::read(path)?;
    let prefix = format!("{name}.bak-");
    let src = Handle::from_path(path);
    if identical_backup_exists(dir, src.as_ref().ok(), &body, |f| {
        in_folder || f.starts_with(&prefix)
    }) {
        return Ok(None);
    }
    let perm = fs::metadata(path)?.permissions();
    let mut n = 0u64;
    if in_folder {
        fs::create_dir_all(dir)?;
        // Past the highest `-<n>` of this second, never a slot pruning freed: the new copy
        // must sort newest, or the next prune would delete it first.
        n = generations(dir, name)
            .iter()
            .filter(|((t, _), _)| *t == ts)
            .map(|((_, n), _)| n + 1)
            .max()
            .unwrap_or(0);
    }
    // `create_new` is O_EXCL: a planted symlink (dangling or live) makes the
    // open fail instead of following it, and two processes cannot share a name.
    loop {
        let bak = if n == 0 {
            dir.join(format!("{name}.bak-{ts}"))
        } else {
            dir.join(format!("{name}.bak-{ts}-{n}"))
        };
        match OpenOptions::new().write(true).create_new(true).open(&bak) {
            Ok(mut dest) => {
                dest.write_all(&body)?;
                dest.set_permissions(perm)?;
                return Ok(Some(bak));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                n = n
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("too many backup name collisions"))?;
            }
            Err(e) => return Err(e),
        }
    }
}

/// True when any distinct regular file in `dir` whose name passes `named` is byte-equal to
/// `body`. Size first. Symlinks and hard links to the source are not backups.
fn identical_backup_exists(
    dir: &Path,
    src: Option<&Handle>,
    body: &[u8],
    named: impl Fn(&str) -> bool,
) -> bool {
    // Without a handle on the source a hard link to it cannot be told from a copy, and
    // counting it as one would skip the only real backup.
    let Some(src) = src else {
        return false;
    };
    // A bare relative path has an empty parent, which `read_dir` rejects.
    let listing = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let Ok(entries) = fs::read_dir(listing) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|e| {
        if !named(&e.file_name().to_string_lossy()) {
            return false;
        }
        let Ok(meta) = fs::symlink_metadata(e.path()) else {
            return false;
        };
        if !meta.is_file() || Handle::from_path(e.path()).is_ok_and(|h| h == *src) {
            return false;
        }
        meta.len() == body.len() as u64 && fs::read(e.path()).is_ok_and(|b| b == body)
    })
}

/// Regular files in `dir` named `<name>.bak-<ts>[-<n>]`, with `(ts, n)` to sort them by.
fn generations(dir: &Path, name: &str) -> Vec<((u64, u64), PathBuf)> {
    let prefix = format!("{name}.bak-");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        // `DirEntry::file_type` does not follow symlinks: a link is never a generation.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let f = e.file_name().into_string().ok()?;
            Some((generation(f.strip_prefix(&prefix)?)?, e.path()))
        })
        .collect()
}

/// `<ts>` or `<ts>-<n>`, the suffix the copy loop writes, as a sortable pair; anything
/// else is `None`.
fn generation(suffix: &str) -> Option<(u64, u64)> {
    let (ts, n) = suffix.split_once('-').unwrap_or((suffix, "0"));
    let num = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    Some((num(ts)?, num(n)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn tmp() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[rstest]
    fn backup_skips_a_missing_file(tmp: TempDir) {
        assert_eq!(backup(&tmp.path().join("nope.toml")).unwrap(), None);
    }

    #[rstest]
    fn backup_skips_a_hundred_preexisting_names_without_clobbering(tmp: TempDir) {
        let dir = tmp.path();
        let path = dir.join("settings.json");
        let ts = 1_700_000_000;
        fs::write(&path, "v0").unwrap();
        let first = backup_at(&path, ts).unwrap().expect("first copy");
        assert_eq!(
            first.file_name().unwrap().to_string_lossy(),
            format!("settings.json.bak-{ts}")
        );

        fs::write(&path, "v1").unwrap();
        for n in 1..100 {
            let slot = dir.join(format!("settings.json.bak-{ts}-{n}"));
            fs::write(&slot, format!("slot-{n}")).unwrap();
        }

        let contents_before: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .map(|e| (e.path(), fs::read_to_string(e.path()).unwrap()))
            .collect();
        assert_eq!(contents_before.len(), 100, "base plus slots 1..=99");

        fs::write(&path, "v2").unwrap();
        let second = backup_at(&path, ts).unwrap().expect("second copy");
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            format!("settings.json.bak-{ts}-100")
        );
        assert_eq!(fs::read_to_string(&second).unwrap(), "v2");

        for (bak_path, content) in contents_before {
            assert_eq!(
                fs::read_to_string(&bak_path).unwrap(),
                content,
                "pre-existing backup must survive: {}",
                bak_path.display()
            );
        }
    }

    #[rstest]
    fn backup_skips_when_an_identical_copy_exists_under_any_name(tmp: TempDir) {
        let dir = tmp.path();
        let path = dir.join("settings.json");
        fs::write(&path, "same").unwrap();
        let first = backup_at(&path, 1).unwrap().expect("first copy");
        assert_eq!(backup_at(&path, 2).unwrap(), None, "same bytes, no copy");
        fs::rename(&first, dir.join("settings.json.bak-9-7")).unwrap();
        assert_eq!(backup_at(&path, 3).unwrap(), None);
        fs::write(&path, "diff").unwrap();
        let third = backup_at(&path, 4)
            .unwrap()
            .expect("changed content is copied");
        assert_eq!(fs::read_to_string(&third).unwrap(), "diff");
        fs::write(dir.join("other.json.bak-1"), "back").unwrap();
        fs::write(&path, "back").unwrap();
        assert!(backup_at(&path, 5).unwrap().is_some(), "prefix is per file");
    }

    #[cfg(unix)]
    #[rstest]
    fn backup_does_not_follow_a_dangling_dest_symlink(tmp: TempDir) {
        use std::os::unix::fs::symlink;
        let dir = tmp.path();
        let src = dir.join("settings.json");
        fs::write(&src, "PAYLOAD\n").unwrap();
        let ts = 1_700_000_000u64;
        let bak = dir.join(format!("settings.json.bak-{ts}"));
        let victim = dir.join("victim-was-missing");
        symlink(&victim, &bak).unwrap();
        let written = backup_at(&src, ts).unwrap().expect("real backup");
        assert_eq!(
            written.file_name().unwrap().to_string_lossy(),
            format!("settings.json.bak-{ts}-1")
        );
        assert_eq!(fs::read_to_string(&written).unwrap(), "PAYLOAD\n");
        assert!(bak.is_symlink(), "planted link must survive");
        assert!(
            !victim.exists(),
            "dangling symlink must not create the target"
        );
    }

    #[cfg(unix)]
    #[rstest]
    fn backup_does_not_treat_a_hardlink_as_an_identical_copy(tmp: TempDir) {
        let dir = tmp.path();
        let src = dir.join("settings.json");
        fs::write(&src, "same-bytes\n").unwrap();
        fs::hard_link(&src, dir.join("settings.json.bak-1")).unwrap();
        let bak = backup_at(&src, 2).unwrap().expect("needs a real copy");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "same-bytes\n");
        fs::write(&src, "REPLACED\n").unwrap();
        assert_eq!(fs::read_to_string(&bak).unwrap(), "same-bytes\n");
        assert_eq!(
            fs::read_to_string(dir.join("settings.json.bak-1")).unwrap(),
            "REPLACED\n",
            "the original hard link tracks the live file"
        );
    }

    #[cfg(unix)]
    #[rstest]
    fn backup_errors_on_a_dangling_source_symlink(tmp: TempDir) {
        use std::os::unix::fs::symlink;
        let dangling = tmp.path().join("settings.json");
        symlink(tmp.path().join("nope"), &dangling).unwrap();
        let err = backup(&dangling).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    fn folder(keep: usize) -> Folder<'static> {
        Folder {
            name: DEFAULT_FOLDER,
            keep,
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }

    fn file_names(paths: &[PathBuf]) -> Vec<String> {
        let mut n: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        n.sort();
        n
    }

    #[rstest]
    fn folder_copies_into_the_subfolder_and_skips_a_missing_file(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        assert_eq!(folder(0).backup(&path).unwrap(), None);
        assert!(!tmp.path().join(DEFAULT_FOLDER).exists(), "nothing created");
        fs::write(&path, "one").unwrap();
        let bak = folder(0).backup_at(&path, 5).unwrap().expect("copy");
        assert_eq!(
            bak,
            tmp.path().join(DEFAULT_FOLDER).join("settings.json.bak-5")
        );
        assert_eq!(fs::read_to_string(bak).unwrap(), "one");
        assert_eq!(names(tmp.path()), [DEFAULT_FOLDER, "settings.json"]);
    }

    #[rstest]
    fn folder_skips_a_hundred_preexisting_names_without_clobbering(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        let dir = tmp.path().join(DEFAULT_FOLDER);
        let ts = 1_700_000_000;
        fs::write(&path, "v0").unwrap();
        let first = folder(0).backup_at(&path, ts).unwrap().expect("first copy");
        assert_eq!(first, dir.join(format!("settings.json.bak-{ts}")));
        fs::write(&path, "v1").unwrap();
        for n in 1..100 {
            fs::write(
                dir.join(format!("settings.json.bak-{ts}-{n}")),
                format!("slot-{n}"),
            )
            .unwrap();
        }
        fs::write(&path, "v2").unwrap();
        let second = folder(0)
            .backup_at(&path, ts)
            .unwrap()
            .expect("second copy");
        assert_eq!(second, dir.join(format!("settings.json.bak-{ts}-100")));
        assert_eq!(fs::read_to_string(&second).unwrap(), "v2");
        assert_eq!(fs::read_to_string(&first).unwrap(), "v0");
        assert_eq!(
            fs::read_to_string(dir.join(format!("settings.json.bak-{ts}-7"))).unwrap(),
            "slot-7"
        );
    }

    #[rstest]
    fn folder_skips_when_any_file_in_it_holds_the_same_bytes(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        let dir = tmp.path().join(DEFAULT_FOLDER);
        fs::write(&path, "same").unwrap();
        let first = folder(0).backup_at(&path, 1).unwrap().expect("first copy");
        assert_eq!(
            folder(0).backup_at(&path, 2).unwrap(),
            None,
            "same bytes, no copy"
        );
        fs::rename(&first, dir.join("settings.json.bak-9-7")).unwrap();
        assert_eq!(folder(0).backup_at(&path, 3).unwrap(), None);
        fs::rename(dir.join("settings.json.bak-9-7"), dir.join("other.txt")).unwrap();
        assert_eq!(
            folder(0).backup_at(&path, 3).unwrap(),
            None,
            "name is ignored"
        );
        fs::write(&path, "diff").unwrap();
        let third = folder(0)
            .backup_at(&path, 4)
            .unwrap()
            .expect("changed content");
        assert_eq!(fs::read_to_string(third).unwrap(), "diff");
        fs::write(dir.join("other.json"), "back").unwrap();
        fs::write(&path, "back").unwrap();
        assert_eq!(
            folder(0).backup_at(&path, 5).unwrap(),
            None,
            "bytes anywhere count"
        );
        fs::write(tmp.path().join("settings.json.bak-1"), "beside").unwrap();
        fs::write(&path, "beside").unwrap();
        assert!(
            folder(0).backup_at(&path, 6).unwrap().is_some(),
            "a sibling outside the folder is not a backup"
        );
    }

    #[cfg(unix)]
    #[rstest]
    fn folder_does_not_follow_a_symlink_planted_at_the_next_slot(tmp: TempDir) {
        use std::os::unix::fs::symlink;
        let src = tmp.path().join("settings.json");
        let dir = tmp.path().join(DEFAULT_FOLDER);
        fs::create_dir(&dir).unwrap();
        fs::write(&src, "PAYLOAD\n").unwrap();
        let victim = tmp.path().join("victim-was-missing");
        let planted = dir.join("settings.json.bak-9");
        symlink(&victim, &planted).unwrap();
        let written = folder(0).backup_at(&src, 9).unwrap().expect("real backup");
        assert_eq!(written, dir.join("settings.json.bak-9-1"));
        assert_eq!(fs::read_to_string(&written).unwrap(), "PAYLOAD\n");
        assert!(planted.is_symlink(), "planted link must survive");
        assert!(
            !victim.exists(),
            "dangling symlink must not create the target"
        );
    }

    #[cfg(unix)]
    #[rstest]
    fn folder_does_not_treat_links_as_identical_copies(tmp: TempDir) {
        use std::os::unix::fs::symlink;
        let src = tmp.path().join("settings.json");
        let dir = tmp.path().join(DEFAULT_FOLDER);
        fs::create_dir(&dir).unwrap();
        fs::write(&src, "same-bytes\n").unwrap();
        fs::hard_link(&src, dir.join("hard")).unwrap();
        symlink(&src, dir.join("soft")).unwrap();
        let bak = folder(0)
            .backup_at(&src, 2)
            .unwrap()
            .expect("needs a real copy");
        fs::write(&src, "REPLACED\n").unwrap();
        assert_eq!(fs::read_to_string(bak).unwrap(), "same-bytes\n");
        assert_eq!(fs::read_to_string(dir.join("hard")).unwrap(), "REPLACED\n");
    }

    #[rstest]
    fn folder_name_must_be_one_plain_component(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        fs::write(&path, "x").unwrap();
        for name in ["", "..", ".", "a/b", "../escape", "/abs"] {
            let f = Folder { name, keep: 0 };
            let err = f.backup_at(&path, 1).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        }
        assert_eq!(names(tmp.path()), ["settings.json"]);
    }

    #[rstest]
    fn a_copy_in_the_same_second_sorts_after_every_kept_one(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        fs::write(&path, "a").unwrap();
        let first = folder(0).backup_at(&path, 7).unwrap().unwrap();
        fs::write(&path, "b").unwrap();
        folder(0).backup_at(&path, 7).unwrap().unwrap();
        fs::remove_file(&first).unwrap();
        fs::write(&path, "c").unwrap();
        let third = folder(0).backup_at(&path, 7).unwrap().unwrap();
        assert!(
            third.ends_with("settings.json.bak-7-2"),
            "{}",
            third.display()
        );
    }

    #[rstest]
    fn prune_keeps_the_newest_generations_of_that_name_only(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        let dir = tmp.path().join(DEFAULT_FOLDER);
        for (i, ts) in [(1, "10"), (2, "10-1"), (3, "10-2"), (4, "11"), (5, "9")] {
            fs::write(&path, format!("v{i}")).unwrap();
            let b = folder(0).backup_at(&path, 0).unwrap().unwrap();
            fs::rename(b, dir.join(format!("settings.json.bak-{ts}"))).unwrap();
        }
        let foreign = [
            "settings.json.bak-old",
            "settings.json.bak-1-x",
            "settings.json.bak-+1",
            "settings.json.bak-",
            "settings.json.bak-1.bak-2",
            "mcp.json.bak-1",
            "notes.txt",
        ];
        for f in foreign {
            fs::write(dir.join(f), f).unwrap();
        }
        fs::create_dir(dir.join("settings.json.bak-3")).unwrap();
        fs::write(&path, "v6").unwrap();
        let kept = folder(3).backup_at(&path, 12).unwrap().unwrap();
        folder(3).prune(&kept);
        let mut want: Vec<String> = foreign.iter().map(|s| s.to_string()).collect();
        want.extend(
            [
                "settings.json.bak-3",
                "settings.json.bak-10-2",
                "settings.json.bak-11",
                "settings.json.bak-12",
            ]
            .map(String::from),
        );
        want.sort();
        assert_eq!(
            names(&dir),
            want,
            "a directory of the right shape is not a candidate"
        );
    }

    #[rstest]
    fn prune_with_keep_zero_keeps_everything(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        for i in 0..4 {
            fs::write(&path, format!("v{i}")).unwrap();
            folder(0)
                .backup(&path)
                .unwrap()
                .expect("new bytes are copied");
            let dir = tmp.path().join(DEFAULT_FOLDER);
            let older = dir.join(format!("settings.json.bak-{i}"));
            fs::write(&older, "old").unwrap();
        }
        let dir = tmp.path().join(DEFAULT_FOLDER);
        let before = names(&dir);
        assert_eq!(before.len(), 8);
        let kept = dir.join(&before[0]);
        folder(0).prune(&kept);
        assert!(folder(0).stale(&dir).is_empty());
        assert_eq!(names(&dir), before);
    }

    #[rstest]
    fn prune_ignores_a_folder_with_another_name_and_a_copy_beside_the_file(tmp: TempDir) {
        let other = tmp.path().join("elsewhere");
        fs::create_dir(&other).unwrap();
        for f in ["a.bak-1", "a.bak-2", "a.bak-3"] {
            fs::write(other.join(f), f).unwrap();
        }
        folder(1).prune(&other.join("a.bak-3"));
        assert_eq!(names(&other).len(), 3, "folder name differs");
        let custom = Folder {
            name: "elsewhere",
            keep: 1,
        };
        assert!(folder(1).stale(&other).is_empty());
        assert_eq!(file_names(&custom.stale(&other)), ["a.bak-1", "a.bak-2"]);
        custom.prune(&other.join("a.bak-3"));
        assert_eq!(names(&other), ["a.bak-3"]);
        fs::write(tmp.path().join("b.bak-1"), "x").unwrap();
        fs::write(tmp.path().join("b.bak-2"), "x").unwrap();
        folder(1).prune(&tmp.path().join("b.bak-2"));
        assert!(tmp.path().join("b.bak-1").exists(), "not inside a folder");
    }

    #[rstest]
    fn prune_never_deletes_the_copy_just_taken(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        fs::write(&path, "future").unwrap();
        let future = folder(1).backup_at(&path, 99).unwrap().unwrap();
        fs::write(&path, "now").unwrap();
        let kept = folder(1).backup_at(&path, 5).unwrap().unwrap();
        folder(1).prune(&kept);
        assert!(kept.exists(), "just taken");
        assert!(!future.exists(), "over the cap");
    }

    #[cfg(unix)]
    #[rstest]
    fn prune_skips_symlinks_and_survives_a_missing_folder(tmp: TempDir) {
        use std::os::unix::fs::symlink;
        let dir = tmp.path().join(DEFAULT_FOLDER);
        fs::create_dir(&dir).unwrap();
        let target = tmp.path().join("precious");
        fs::write(&target, "keep me").unwrap();
        symlink(&target, dir.join("a.bak-1")).unwrap();
        fs::write(dir.join("a.bak-2"), "x").unwrap();
        fs::write(dir.join("a.bak-3"), "x").unwrap();
        folder(1).prune(&dir.join("a.bak-3"));
        assert_eq!(names(&dir), ["a.bak-1", "a.bak-3"]);
        assert!(target.exists());
        folder(1).prune(&tmp.path().join("gone").join(DEFAULT_FOLDER).join("a.bak-1"));
    }

    #[rstest]
    fn backup_caps_generations_per_file(tmp: TempDir) {
        let path = tmp.path().join("settings.json");
        let dir = tmp.path().join(DEFAULT_FOLDER);
        for i in 0..4 {
            fs::write(&path, format!("v{i}")).unwrap();
            folder(2)
                .backup(&path)
                .unwrap()
                .expect("new bytes are copied");
        }
        let bodies: Vec<String> = names(&dir)
            .iter()
            .map(|n| fs::read_to_string(dir.join(n)).unwrap())
            .collect();
        assert_eq!(names(&dir).len(), 2);
        assert!(bodies.contains(&"v3".to_string()) && bodies.contains(&"v2".to_string()));
        assert_eq!(
            folder(2).backup(&path).unwrap(),
            None,
            "identical: no copy, no prune"
        );
        assert_eq!(names(&dir).len(), 2);
    }

    #[rstest]
    fn stale_lists_only_generations_past_the_cap(tmp: TempDir) {
        let dir = tmp.path().join(DEFAULT_FOLDER);
        fs::create_dir(&dir).unwrap();
        for f in [
            "a.json.bak-1",
            "a.json.bak-2-1",
            "a.json.bak-2",
            "b.json.bak-7",
            "a.json.bak-x",
        ] {
            fs::write(dir.join(f), "x").unwrap();
        }
        let stale = |keep| file_names(&folder(keep).stale(&dir));
        assert_eq!(stale(1), ["a.json.bak-1", "a.json.bak-2"]);
        assert_eq!(stale(2), ["a.json.bak-1"]);
        assert!(stale(0).is_empty() && stale(3).is_empty());
        assert!(
            folder(1).stale(tmp.path()).is_empty(),
            "not a _backup folder"
        );
        let oldest_first = folder(1).stale(&dir);
        assert_eq!(oldest_first[0], dir.join("a.json.bak-1"));
    }
}
