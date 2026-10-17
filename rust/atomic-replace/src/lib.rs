//! Replace a file atomically.
//!
//! The new bytes go to a sibling temp file that is renamed over the target, so
//! a reader, a crash or a full disk never sees a half-written file: the target
//! holds either the old bytes or the new ones.
//!
//! ```no_run
//! # fn main() -> std::io::Result<()> {
//! use std::io::Write;
//!
//! atomic_replace::write("state.json", b"{}\n")?;
//!
//! atomic_replace::Options::new()
//!     .create_parent(true)
//!     .durable(false)
//!     .write_with("cache/index.bin", |file| file.write_all(b"index"))?;
//! # Ok(())
//! # }
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Replace `path` with `bytes` using the default [`Options`].
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    Options::new().write(path, bytes)
}

/// How [`Options::write`] and [`Options::write_with`] replace a file.
///
/// The defaults are the safe ones: fsync, keep permissions, keep symlinks,
/// and do not create missing directories.
#[derive(Clone, Debug)]
pub struct Options {
    durable: bool,
    preserve_permissions: bool,
    follow_symlinks: bool,
    create_parent: bool,
    #[cfg(unix)]
    mode: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

impl Options {
    pub fn new() -> Self {
        Self {
            durable: true,
            preserve_permissions: true,
            follow_symlinks: true,
            create_parent: false,
            #[cfg(unix)]
            mode: None,
        }
    }

    /// Fsync the file before the rename and its directory after it (default
    /// on). Without the file fsync the kernel may record the rename and lose
    /// the bytes, leaving an empty file after a crash.
    pub fn durable(mut self, on: bool) -> Self {
        self.durable = on;
        self
    }

    /// Copy an existing target's permissions onto the new file (default on),
    /// so a 0600 file stays 0600. On Windows this clears the target's
    /// read-only bit instead, because the rename cannot replace a read-only
    /// file.
    pub fn preserve_permissions(mut self, on: bool) -> Self {
        self.preserve_permissions = on;
        self
    }

    /// When the target is a symlink, write the file it points to and keep the
    /// link (default on). Off replaces the link itself with a plain file.
    pub fn follow_symlinks(mut self, on: bool) -> Self {
        self.follow_symlinks = on;
        self
    }

    /// Create the target's missing parent directories (default off).
    pub fn create_parent(mut self, on: bool) -> Self {
        self.create_parent = on;
        self
    }

    /// Unix mode of the new file. An existing target's permissions still win
    /// while [`preserve_permissions`](Self::preserve_permissions) is on.
    #[cfg(unix)]
    pub fn mode(mut self, mode: u32) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Replace `path` with `bytes`.
    pub fn write(&self, path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
        self.write_with(path, |file| file.write_all(bytes.as_ref()))
    }

    /// Replace `path` with whatever `fill` writes into the temp file.
    pub fn write_with<F>(&self, path: impl AsRef<Path>, fill: F) -> io::Result<()>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
    {
        let path = path.as_ref();
        if path.file_name().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} names no file", path.display()),
            ));
        }
        if self.create_parent {
            if let Some(dir) = parent(path) {
                fs::create_dir_all(dir)?;
            }
        }
        let target = if self.follow_symlinks {
            link_target(path)?
        } else {
            path.to_path_buf()
        };
        let (tmp, mut file) = self.create_temp(&target)?;
        let result = self.fill_and_swap(&target, &tmp, &mut file, fill);
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    fn fill_and_swap<F>(
        &self,
        target: &Path,
        tmp: &Path,
        file: &mut File,
        fill: F,
    ) -> io::Result<()>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
    {
        fill(file)?;
        file.flush()?;
        if self.preserve_permissions {
            keep_permissions(target, tmp)?;
        }
        if self.durable {
            file.sync_all()?;
        }
        fs::rename(tmp, target)?;
        if self.durable {
            if let Some(dir) = parent(target) {
                // Best effort: some filesystems (and Windows) refuse to fsync
                // a directory, and the bytes are already safe by then.
                let _ = sync_dir(dir);
            }
        }
        Ok(())
    }

    /// A fresh temp file beside `target`. `create_new` with pid, time and a
    /// process-wide counter in the name, so two writers never share a file
    /// and a leftover from a crashed run is never reused.
    fn create_temp(&self, target: &Path) -> io::Result<(PathBuf, File)> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = parent(target).unwrap_or(Path::new("."));
        let name = target.file_name().unwrap_or_default().to_string_lossy();
        let mut last = None;
        for _ in 0..8 {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let tmp = dir.join(format!(".{name}.tmp-{}-{nanos}-{n}", std::process::id()));
            let mut open = OpenOptions::new();
            open.write(true).create_new(true);
            #[cfg(unix)]
            if let Some(mode) = self.mode {
                use std::os::unix::fs::OpenOptionsExt;
                open.mode(mode);
            }
            match open.open(&tmp) {
                Ok(file) => return Ok((tmp, file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("no free temp file name")))
    }
}

/// Fsync a directory so a rename or create inside it survives a crash.
pub fn sync_dir(dir: impl AsRef<Path>) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// `Path::parent` returns `Some("")` for a bare file name; that means the
/// current directory.
fn parent(path: &Path) -> Option<&Path> {
    match path.parent() {
        Some(p) if p.as_os_str().is_empty() => Some(Path::new(".")),
        other => other,
    }
}

/// The file `path` resolves to. `canonicalize` fails on a dangling link (a
/// dotfile manager's link whose target is not there yet), and the rename then
/// replaced the link with a plain file; following the links by hand writes the
/// target and keeps the link. A cycle is an error: replacing one of its links
/// with a file would silently break the others.
fn link_target(path: &Path) -> io::Result<PathBuf> {
    let mut p = path.to_path_buf();
    // 40: the kernel's own ELOOP limit.
    for _ in 0..40 {
        if let Ok(real) = fs::canonicalize(&p) {
            return Ok(real);
        }
        match fs::read_link(&p) {
            Ok(next) => p = p.parent().unwrap_or(Path::new("")).join(next),
            Err(_) => return Ok(p),
        }
    }
    Err(io::Error::other(format!(
        "{}: too many levels of symbolic links",
        path.display()
    )))
}

#[cfg(unix)]
fn keep_permissions(target: &Path, tmp: &Path) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(meta) => fs::set_permissions(tmp, meta.permissions()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// `MOVEFILE_REPLACE_EXISTING` cannot replace a read-only file; drop that bit
/// on the target rather than copying it onto the temp file.
#[cfg(windows)]
// Windows-only, so the world-writable risk the lint names for Unix cannot arise here.
#[allow(clippy::permissions_set_readonly_false)]
fn keep_permissions(target: &Path, _tmp: &Path) -> io::Result<()> {
    let Ok(meta) = fs::metadata(target) else {
        return Ok(());
    };
    let mut perms = meta.permissions();
    if perms.readonly() {
        perms.set_readonly(false);
        fs::set_permissions(target, perms)?;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn keep_permissions(_target: &Path, _tmp: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn creates_and_replaces_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("state.json");
        write(&f, "one").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "one");
        write(&f, "two").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "two");
        assert_eq!(entries(dir.path()), ["state.json"]);
    }

    #[test]
    fn failed_fill_keeps_the_old_file_and_removes_the_temp() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a");
        write(&f, "old").unwrap();
        let err = Options::new()
            .write_with(&f, |file| {
                file.write_all(b"half")?;
                Err(io::Error::other("boom"))
            })
            .unwrap_err();
        assert_eq!(err.to_string(), "boom");
        assert_eq!(fs::read_to_string(&f).unwrap(), "old");
        assert_eq!(entries(dir.path()), ["a"]);
    }

    #[test]
    fn missing_parent_fails_unless_asked_to_create_it() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x/y/z.toml");
        let err = write(&f, "v").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        Options::new().create_parent(true).write(&f, "v").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "v");
    }

    #[test]
    fn path_without_a_file_name_is_refused() {
        let err = write("..", "v").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn not_durable_still_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("c");
        Options::new().durable(false).write(&f, "fast").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "fast");
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::{symlink, PermissionsExt};

        fn mode(p: &Path) -> u32 {
            fs::metadata(p).unwrap().permissions().mode() & 0o777
        }

        #[test]
        fn keeps_the_old_mode() {
            let dir = tempfile::tempdir().unwrap();
            let f = dir.path().join("secret");
            fs::write(&f, "old").unwrap();
            fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
            write(&f, "new").unwrap();
            assert_eq!(mode(&f), 0o600);
        }

        #[test]
        fn mode_applies_to_a_new_file_and_yields_to_an_existing_one() {
            let dir = tempfile::tempdir().unwrap();
            let f = dir.path().join("key");
            Options::new().mode(0o600).write(&f, "k").unwrap();
            assert_eq!(mode(&f), 0o600);
            fs::set_permissions(&f, fs::Permissions::from_mode(0o640)).unwrap();
            Options::new().mode(0o600).write(&f, "k2").unwrap();
            assert_eq!(mode(&f), 0o640);
            Options::new()
                .mode(0o600)
                .preserve_permissions(false)
                .write(&f, "k3")
                .unwrap();
            assert_eq!(mode(&f), 0o600);
        }

        #[test]
        fn writes_through_a_symlink_and_keeps_it() {
            let dir = tempfile::tempdir().unwrap();
            let real = dir.path().join("real.json");
            let link = dir.path().join("link.json");
            fs::write(&real, "old").unwrap();
            symlink("real.json", &link).unwrap();
            write(&link, "new").unwrap();
            assert!(fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read_to_string(&real).unwrap(), "new");
        }

        #[test]
        fn creates_the_target_of_a_dangling_symlink() {
            let dir = tempfile::tempdir().unwrap();
            let link = dir.path().join("link");
            symlink("missing", &link).unwrap();
            write(&link, "made").unwrap();
            assert!(fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                fs::read_to_string(dir.path().join("missing")).unwrap(),
                "made"
            );
        }

        #[test]
        fn not_following_replaces_the_link_itself() {
            let dir = tempfile::tempdir().unwrap();
            let real = dir.path().join("real");
            let link = dir.path().join("link");
            fs::write(&real, "old").unwrap();
            symlink(&real, &link).unwrap();
            Options::new()
                .follow_symlinks(false)
                .write(&link, "new")
                .unwrap();
            assert!(fs::symlink_metadata(&link).unwrap().is_file());
            assert_eq!(fs::read_to_string(&real).unwrap(), "old");
        }

        #[test]
        fn symlink_cycle_ends() {
            let dir = tempfile::tempdir().unwrap();
            let a = dir.path().join("a");
            let b = dir.path().join("b");
            symlink(&b, &a).unwrap();
            symlink(&a, &b).unwrap();
            assert!(write(&a, "v").is_err());
            assert_eq!(entries(dir.path()), ["a", "b"]);
        }
    }

    #[cfg(windows)]
    #[test]
    fn replaces_a_read_only_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("ro.json");
        fs::write(&f, "old").unwrap();
        let mut perms = fs::metadata(&f).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&f, perms).unwrap();
        write(&f, "new").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "new");
    }
}
