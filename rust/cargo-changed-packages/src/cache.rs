//! On-disk cache of `cargo metadata` output, keyed by everything that can change the package
//! graph. It lives apart from the graph logic so the "any doubt means recompute" rule has
//! one owner: every failure here (unreadable file, odd path, hash mismatch) yields a miss or
//! `Disabled`, never a wrong graph.

use std::collections::hash_map::DefaultHasher;
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Bump when the entry layout or the key recipe changes; old entries then miss.
const FORMAT: &str = "cargo-changed-packages-metadata-v1";

/// What the cache did for one graph load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheStatus {
    /// The metadata JSON came from the cache; `cargo metadata` did not run.
    Hit,
    /// No valid entry existed; `cargo metadata` ran and its output was stored.
    Miss,
    /// `cargo metadata` ran but its output could not be stored (the reason).
    MissNotStored(String),
    /// Caching was off, or a cache key could not be computed safely; `cargo metadata` ran.
    Disabled,
}

/// A cache entry for one workspace root.
pub(crate) struct Entry {
    file: PathBuf,
    key: u64,
}

impl Entry {
    /// `None` when the key cannot be computed: any doubt means no cache.
    pub(crate) fn open(dir: &Path, root: &Path, cargo: &OsStr) -> Option<Self> {
        let root = root.canonicalize().ok()?;
        let key = fingerprint(&root, cargo).ok()?;
        // One file per workspace root, overwritten on change, so the directory stays small.
        let file = dir.join(format!("metadata-{:016x}.txt", hash_of(root.as_os_str())));
        Some(Self { file, key })
    }

    /// The cached metadata JSON, or `None` on any mismatch or read problem.
    pub(crate) fn read(&self) -> Option<String> {
        let text = fs::read_to_string(&self.file).ok()?;
        let (header, rest) = text.split_once("\n\n")?;
        let mut lines = header.lines();
        if lines.next()? != format!("{FORMAT} {:016x}", self.key) {
            return None;
        }
        // Manifests outside the workspace (path dependencies) are not in the key, so each
        // one the graph named is checked again against its recorded hash.
        for line in lines {
            let (hash, path) = line.split_once('\t')?;
            if file_hash(Path::new(path)).ok()? != u64::from_str_radix(hash, 16).ok()? {
                return None;
            }
        }
        Some(rest.to_owned())
    }

    /// Store `json`; `outside` are the manifests of path packages outside the workspace root.
    pub(crate) fn write(&self, json: &str, outside: &[PathBuf]) -> Result<(), String> {
        let mut text = format!("{FORMAT} {:016x}\n", self.key);
        for path in outside {
            let shown = path.to_str().filter(|p| !p.contains(['\t', '\n']));
            let shown = shown.ok_or("a manifest path cannot be recorded")?;
            let hash = file_hash(path).map_err(|e| format!("{}: {e}", path.display()))?;
            text.push_str(&format!("{hash:016x}\t{shown}\n"));
        }
        text.push('\n');
        text.push_str(json);
        let dir = self.file.parent().ok_or("cache file has no directory")?;
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        // Write then rename so a concurrent reader never sees half an entry.
        let tmp = self
            .file
            .with_extension(format!("tmp{}", std::process::id()));
        fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.file).map_err(|e| format!("{}: {e}", self.file.display()))
    }
}

fn hash_of<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn file_hash(path: &Path) -> io::Result<u64> {
    Ok(hash_of(&fs::read(path)?))
}

/// Hash of the tool version, `cargo --version`, and the content of `Cargo.lock`, every
/// `Cargo.toml` under `root`, `.cargo/config*` and `rust-toolchain*`. A repository without
/// `Cargo.lock` is not cached: its resolution is not pinned.
fn fingerprint(root: &Path, cargo: &OsStr) -> io::Result<u64> {
    let mut hasher = DefaultHasher::new();
    FORMAT.hash(&mut hasher);
    let version = Command::new(cargo).arg("--version").output()?;
    if !version.status.success() {
        return Err(io::Error::other("cargo --version failed"));
    }
    cargo.hash(&mut hasher);
    version.stdout.hash(&mut hasher);
    let lock = root.join("Cargo.lock");
    fs::read(&lock)?.hash(&mut hasher);
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort();
    for relative in files {
        relative.hash(&mut hasher);
        fs::read(root.join(&relative))?.hash(&mut hasher);
    }
    Ok(hasher.finish())
}

/// Collects root-relative manifest and config files. Directories `target`, `.git` and
/// `node_modules` are skipped and symlinks are not followed.
fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if !matches!(name.as_ref(), "target" | ".git" | "node_modules") {
                collect(root, &path, out)?;
            }
        } else if kind.is_file() {
            let in_cargo_dir = dir.file_name().is_some_and(|d| d == ".cargo");
            let relevant = name == "Cargo.toml"
                || (dir == root && name.starts_with("rust-toolchain"))
                || (in_cargo_dir && name.starts_with("config"));
            if relevant {
                let relative = path.strip_prefix(root).map_err(io::Error::other)?;
                out.push(relative.to_path_buf());
            }
        }
    }
    Ok(())
}
