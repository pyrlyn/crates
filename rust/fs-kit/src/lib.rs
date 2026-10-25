//! One entry point for file and directory work, so every project spells a path, writes a
//! file and walks a tree the same way.
//!
//! - [`path`]: path identity without the Windows `\\?\` prefix, case-folded on Windows, and
//!   a lexical join and clean that need no disk.
//! - [`write`]: atomic replace through a sibling temp file, following symlinks and keeping
//!   permissions; write only when the bytes differ.
//! - [`read`]: read text where a missing file is empty.
//! - [`tree`]: disk usage of a tree, copy a tree, remove whatever is at a path.
//! - [`backup`]: sibling `<name>.bak-<unix-seconds>` copies before a file is replaced.
//!
//! Every error is an [`std::io::Error`] of the original kind that names its path; [`split`]
//! and [`raw_os_error`] reach the original error behind it.

pub mod backup;
pub mod path;
pub mod read;
pub mod tree;
pub mod write;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The original error and the path it happened at. Every error fs-kit returns is an
/// [`io::Error`] of the original kind carrying one of these, so `?` and `kind()` work as
/// usual and [`split`] gives the path and the original error, OS code included, back.
#[derive(Debug)]
pub struct PathError {
    pub path: PathBuf,
    pub source: io::Error,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// The path and the original error inside an fs-kit error; no path for any other error,
/// which comes back unchanged. A caller with its own path-carrying error type uses this
/// rather than printing the path twice.
pub fn split(err: io::Error) -> (Option<PathBuf>, io::Error) {
    if !err.get_ref().is_some_and(|e| e.is::<PathError>()) {
        return (None, err);
    }
    let kind = err.kind();
    match err.into_inner().map(|e| e.downcast::<PathError>()) {
        Some(Ok(e)) => (Some(e.path), e.source),
        // Unreachable: the payload was checked above. Keep the kind rather than panic.
        _ => (None, io::Error::from(kind)),
    }
}

/// The OS error code behind `err`, also when fs-kit wrapped it with a path.
pub fn raw_os_error(err: &io::Error) -> Option<i32> {
    err.raw_os_error().or_else(|| {
        err.get_ref()?
            .downcast_ref::<PathError>()?
            .source
            .raw_os_error()
    })
}

/// `err` with `path` attached, keeping its kind so a caller can still match on it.
pub(crate) fn at(path: &Path, err: io::Error) -> io::Error {
    io::Error::new(
        err.kind(),
        PathError {
            path: path.to_path_buf(),
            source: err,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapped_error_keeps_its_kind_os_code_and_path() {
        let dir = std::env::temp_dir().join(format!("fs-kit-err-{}", std::process::id()));
        let missing = dir.join("nope");
        let bare = std::fs::read(&missing).unwrap_err();
        let err = std::fs::read(&missing)
            .map_err(|e| at(&missing, e))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(err.to_string(), format!("{}: {bare}", missing.display()));
        let code = raw_os_error(&err);
        assert!(code.is_some(), "the OS code survives the wrap");
        let (path, inner) = split(err);
        assert_eq!(path, Some(missing));
        assert_eq!(inner.raw_os_error(), code);
        let plain = io::Error::other("x");
        let (path, inner) = split(plain);
        assert_eq!((path, inner.to_string()), (None, "x".to_string()));
    }
}
