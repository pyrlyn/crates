// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Copy a file to `<name>.bak-<unix-seconds>` beside it before replacing it.
//!
//! Shared by ketch (`config reset`) and rtok (agent host setup). A missing
//! file, or a sibling `.bak-*` that already holds the same bytes, is `Ok(None)`
//! — a second reset or setup must not grow a stack of identical undos.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Copy `path` to `<name>.bak-<unix-seconds>` beside it.
///
/// `None` when there is no file yet, or when a `<name>.bak-*` sibling already
/// holds the same bytes.
pub fn backup(path: &Path) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    backup_at(path, ts)
}

/// [`backup`] with the clock passed in, so a test can pin the second.
pub fn backup_at(path: &Path, ts: u64) -> std::io::Result<Option<PathBuf>> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let body = fs::read(path)?;
    if identical_backup_exists(path, &name, &body) {
        return Ok(None);
    }
    // Two commands inside one second would otherwise share a name and the
    // first copy would go.
    let mut n = 0u32;
    let mut bak = path.with_file_name(format!("{name}.bak-{ts}"));
    while bak.exists() {
        n += 1;
        bak = path.with_file_name(format!("{name}.bak-{ts}-{n}"));
    }
    fs::copy(path, &bak)?;
    Ok(Some(bak))
}

/// True when any `<name>.bak-*` beside `path` is byte-equal to `body`.
fn identical_backup_exists(path: &Path, name: &str, body: &[u8]) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let prefix = format!("{name}.bak-");
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .any(|e| {
            e.metadata().is_ok_and(|m| m.len() == body.len() as u64)
                && fs::read(e.path()).is_ok_and(|b| b == body)
        })
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
}
