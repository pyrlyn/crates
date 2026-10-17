// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: MIT OR Apache-2.0

//! SHA-256 over a package tree. One function, so a grant decided against a
//! digest and the bytes later copied into `versions/<digest12>/` can be
//! checked with the same computation.

use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

/// SHA-256 over the whole package tree: `(relative path, length, bytes)` for
/// every regular file under `dir`, sorted by path, `/`-joined so the digest
/// does not depend on the host's path separator. Symlinks are skipped, both
/// as files and as directories, so a link out of the package never
/// contributes bytes.
///
/// The hash of an empty tree is the SHA-256 of no updates. Paths that are
/// not valid Unicode are hashed through lossy replacement, which is part of
/// the digest a caller must reproduce.
pub fn package_digest(dir: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for rel in &files {
        let bytes = fs::read(dir.join(rel))?;
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_dir() {
            walk(root, &path, out)?;
        } else if ty.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
