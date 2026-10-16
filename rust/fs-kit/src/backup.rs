//! Copy a file to `<name>.bak-<unix-seconds>` beside it before replacing it. A missing file,
//! or a sibling `.bak-*` that already holds the same bytes, is `Ok(None)`: a second reset or
//! setup must not grow a stack of identical undos.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Copy `path` to `<name>.bak-<unix-seconds>` beside it.
///
/// `None` when there is no file yet, or when a `<name>.bak-*` sibling already holds the same
/// bytes.
pub fn backup(path: &Path) -> io::Result<Option<PathBuf>> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    backup_at(path, ts)
}

/// [`backup`] with the clock passed in, so a test can pin the second.
pub fn backup_at(path: &Path, ts: u64) -> io::Result<Option<PathBuf>> {
    let Some(body) = crate::read::bytes_if_exists(path)? else {
        return Ok(None);
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if identical_backup_exists(path, &name, &body) {
        return Ok(None);
    }
    // `create_new` claims the name: two commands inside one second cannot pick the same one
    // and truncate the first copy, and a planted link at the name is never followed.
    let mut n = 0u32;
    loop {
        let bak = if n == 0 {
            path.with_file_name(format!("{name}.bak-{ts}"))
        } else {
            path.with_file_name(format!("{name}.bak-{ts}-{n}"))
        };
        match OpenOptions::new().write(true).create_new(true).open(&bak) {
            Ok(file) => return write_copy(file, &bak, &body).map(|()| Some(bak)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(crate::at(&bak, e)),
        }
    }
}

fn write_copy(mut file: File, bak: &Path, body: &[u8]) -> io::Result<()> {
    let result = io::Write::write_all(&mut file, body).and_then(|()| file.sync_all());
    if let Err(e) = result {
        drop(file);
        let _ = fs::remove_file(bak);
        return Err(crate::at(bak, e));
    }
    Ok(())
}

/// True when any `<name>.bak-*` beside `path` is byte-equal to `body`.
fn identical_backup_exists(path: &Path, name: &str, body: &[u8]) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let prefix = format!("{name}.bak-");
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .any(|e| {
            e.metadata().is_ok_and(|m| m.len() == body.len() as u64)
                && fs::read(e.path()).is_ok_and(|b| b == body)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fs-kit-bak-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn backup_skips_a_missing_file() {
        let dir = tmp("missing");
        assert_eq!(backup(&dir.join("nope.toml")).unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_skips_taken_names_without_clobbering() {
        let dir = tmp("names");
        let path = dir.join("settings.json");
        let ts = 1_700_000_000;
        fs::write(&path, "v0").unwrap();
        let first = backup_at(&path, ts).unwrap().expect("first copy");
        assert_eq!(first, dir.join(format!("settings.json.bak-{ts}")));
        for n in 1..100 {
            fs::write(
                dir.join(format!("settings.json.bak-{ts}-{n}")),
                format!("slot-{n}"),
            )
            .unwrap();
        }
        fs::write(&path, "v2").unwrap();
        let second = backup_at(&path, ts).unwrap().expect("second copy");
        assert_eq!(second, dir.join(format!("settings.json.bak-{ts}-100")));
        assert_eq!(fs::read_to_string(&second).unwrap(), "v2");
        assert_eq!(fs::read_to_string(&first).unwrap(), "v0");
        assert_eq!(
            fs::read_to_string(dir.join(format!("settings.json.bak-{ts}-7"))).unwrap(),
            "slot-7"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_skips_when_an_identical_copy_exists_under_any_name() {
        let dir = tmp("identical");
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
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_link_at_the_backup_name_is_skipped_not_followed() {
        let dir = tmp("planted");
        let path = dir.join("settings.json");
        fs::write(&path, "secret").unwrap();
        let victim = dir.join("victim");
        fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("settings.json.bak-5")).unwrap();
        let bak = backup_at(&path, 5)
            .unwrap()
            .expect("copy under the next name");
        assert_eq!(bak, dir.join("settings.json.bak-5-1"));
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
        fs::remove_dir_all(&dir).unwrap();
    }
}
