//! One entry point for file and directory work, so every project spells a path, writes a
//! file and walks a tree the same way.
//!
//! - [`path`]: path identity without the Windows `\\?\` prefix, case-folded on Windows, and
//!   a lexical join that needs no disk.
//! - [`write`]: atomic replace through a sibling temp file, following symlinks and keeping
//!   permissions; write only when the bytes differ.
//! - [`read`]: read text where a missing file is empty.
//! - [`tree`]: disk usage of a tree, copy a tree, remove whatever is at a path.
//! - [`backup`]: sibling `<name>.bak-<unix-seconds>` copies before a file is replaced.

pub mod backup;
pub mod path;
pub mod read;
pub mod tree;
pub mod write;

use std::io;
use std::path::Path;

/// `err` with `path` in its message, keeping its kind so a caller can still match on it.
pub(crate) fn at(path: &Path, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{}: {err}", path.display()))
}
