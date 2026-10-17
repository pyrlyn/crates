//! Content-addressed artifact store.
//!
//! Layout under the cache root:
//!
//! ```text
//! .lock                         exclusive lock held while building
//! artifacts/<k[0..2]>/<k>/      one entry per cache key
//!     plugin.<ext>              the built artifact (immutable)
//!     entry.json                metadata incl. sha256
//!     verified.json             file stat the sha256 was last checked against
//! target/                       shared cargo target dir (cargo's own cache)
//! git/<url-hash>-<sha>/         git checkouts
//! loaded/                       shadow copies of loaded native libraries
//! compiled/<engine>/            machine code of Wasm modules, by module sha256
//! meta/                         memoised `rustc -vV`, linker and probe results
//! ```

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use plugwright_core::error::{PluginError, Result, io_err};

/// Metadata stored next to every cached artifact.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub id: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
    pub source: String,
    pub target: String,
    pub built_at_unix: u64,
}

/// A built (or cached) plugin artifact.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub path: PathBuf,
    pub sha256: String,
    /// Cache key; `None` for prebuilt artifacts loaded straight from disk.
    pub key: Option<String>,
    /// `true` when served from the cache without running cargo.
    pub cached: bool,
}

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

/// Exclusive cache lock; released on drop.
pub struct CacheLock(File);

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        for d in ["artifacts", "target", "git", "loaded", "meta"] {
            let p = root.join(d);
            std::fs::create_dir_all(&p).map_err(io_err(&p))?;
        }
        Ok(Cache { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }
    pub fn git_dir(&self) -> PathBuf {
        self.root.join("git")
    }
    pub fn shadow_dir(&self) -> PathBuf {
        self.root.join("loaded")
    }
    pub fn compiled_dir(&self) -> PathBuf {
        self.root.join("compiled")
    }

    /// Block until this process holds the cache lock.
    pub fn lock(&self) -> Result<CacheLock> {
        let p = self.root.join(".lock");
        let f = File::options().create(true).truncate(false).write(true).open(&p).map_err(io_err(&p))?;
        f.lock().map_err(io_err(&p))?;
        Ok(CacheLock(f))
    }

    fn entry_dir(&self, key: &str) -> PathBuf {
        self.root.join("artifacts").join(&key[..2]).join(key)
    }

    /// Return the cached artifact for `key`, verifying its sha256.
    /// A corrupted entry is evicted and reported as a miss.
    ///
    /// The file is rehashed only when its stat differs from the one recorded
    /// at the last verification (see [`sha256_file`] on why that suffices);
    /// a warm lookup costs two small reads and a stat, not a pass over a
    /// large library.
    pub fn lookup(&self, key: &str) -> Result<Option<Artifact>> {
        let dir = self.entry_dir(key);
        let meta = dir.join("entry.json");
        let Ok(raw) = std::fs::read(&meta) else { return Ok(None) };
        let entry: CacheEntry = match serde_json::from_slice(&raw) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(key, error = %e, "corrupt cache entry; evicting");
                let _ = std::fs::remove_dir_all(&dir);
                return Ok(None);
            }
        };
        let path = dir.join(&entry.file);
        let hit =
            |sha256: String| Ok(Some(Artifact { path: path.clone(), sha256, key: Some(key.to_owned()), cached: true }));
        let verified = dir.join("verified.json");
        let Ok(id) = file_id(&path) else {
            let _ = std::fs::remove_dir_all(&dir);
            return Ok(None);
        };
        if let Ok(raw) = std::fs::read(&verified)
            && let Ok(v) = serde_json::from_slice::<Verified>(&raw)
            && v.sha256 == entry.sha256
            && v.stamp == id.stamp
        {
            return hit(entry.sha256);
        }
        let hashed_at = unix_ns(std::time::SystemTime::now());
        match hash_file(&path) {
            Ok(actual) if actual == entry.sha256 => {
                // Recorded only once the times are settled: before that a later
                // rewrite could leave the stat unchanged.
                if settled(&id.stamp, hashed_at)
                    && let Ok(json) = serde_json::to_vec(&Verified { sha256: actual.clone(), stamp: id.stamp })
                {
                    let tmp = dir.join(format!(
                        ".verified-{}-{}",
                        std::process::id(),
                        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
                    ));
                    if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &verified).is_err() {
                        let _ = std::fs::remove_file(&tmp);
                    }
                }
                hit(actual)
            }
            Ok(actual) => {
                tracing::warn!(key, expected = %entry.sha256, %actual, "cached artifact sha256 mismatch; evicting");
                let _ = std::fs::remove_dir_all(&dir);
                Ok(None)
            }
            Err(_) => {
                let _ = std::fs::remove_dir_all(&dir);
                Ok(None)
            }
        }
    }

    /// Copy `built` into the store under `key` atomically (temp dir + rename).
    pub fn store(&self, key: &str, built: &Path, mut entry: CacheEntry) -> Result<Artifact> {
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = self.root.join("artifacts").join(format!(".tmp-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(io_err(&tmp))?;
        let ext = built.extension().and_then(|e| e.to_str()).unwrap_or("bin");
        entry.file = format!("plugin.{ext}");
        let file = tmp.join(&entry.file);
        std::fs::copy(built, &file).map_err(io_err(&file))?;
        entry.sha256 = sha256_file(&file)?;
        entry.size = std::fs::metadata(&file).map_err(io_err(&file))?.len();
        let meta = tmp.join("entry.json");
        std::fs::write(&meta, serde_json::to_vec_pretty(&entry)?).map_err(io_err(&meta))?;

        let dst = self.entry_dir(key);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        if dst.exists() {
            let _ = std::fs::remove_dir_all(&dst);
        }
        std::fs::rename(&tmp, &dst).map_err(io_err(&dst))?;
        Ok(Artifact { path: dst.join(&entry.file), sha256: entry.sha256, key: Some(key.to_owned()), cached: false })
    }

    /// Remove every artifact whose key is not in `keep`. Returns the number removed.
    pub fn gc(&self, keep: &HashSet<String>) -> Result<usize> {
        let _lock = self.lock()?;
        let mut removed = 0;
        let artifacts = self.root.join("artifacts");
        for shard in std::fs::read_dir(&artifacts).map_err(io_err(&artifacts))?.flatten() {
            let Ok(entries) = std::fs::read_dir(shard.path()) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !keep.contains(&name) && std::fs::remove_dir_all(e.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        tracing::info!(removed, "cache gc finished");
        Ok(removed)
    }
}

/// sha256 of a file, remembered for this process while the file's identity
/// and change times stay the same: reloading a plugin does not rehash a large
/// archive. Any write moves `ctime` (Unix) or the write time (Windows), and
/// neither can be set back by an unprivileged writer on Unix.
///
/// Times are only as fine as a kernel tick (whole seconds on some file
/// systems), so two writes close together can leave them equal; a file whose
/// times are not [`SETTLED`] older than the hash is hashed again every time.
pub fn sha256_file(path: &Path) -> Result<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static MEMO: OnceLock<Mutex<HashMap<FileId, String>>> = OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);
    let id = file_id(path)?;
    if let Some(h) = memo.lock().unwrap_or_else(|p| p.into_inner()).get(&id) {
        return Ok(h.clone());
    }
    let hashed_at = unix_ns(std::time::SystemTime::now());
    let h = hash_file(path)?;
    if !settled(&id.stamp, hashed_at) {
        return Ok(h);
    }
    // The identity is taken before hashing: a write during hashing changes it,
    // so a stale hash can only be stored under an identity that no longer exists.
    let mut m = memo.lock().unwrap_or_else(|p| p.into_inner());
    if m.len() >= 4096 {
        m.clear();
    }
    m.insert(id, h.clone());
    Ok(h)
}

/// How much older than a hash a file's times must be for the hash to be
/// remembered: above the coarsest timestamp granularity in use (FAT: 2 s).
const SETTLED: std::time::Duration = std::time::Duration::from_secs(3);

fn settled(stamp: &Stamp, hashed_at: i128) -> bool {
    stamp.times.iter().all(|&t| t + SETTLED.as_nanos() as i128 <= hashed_at)
}

fn unix_ns(t: std::time::SystemTime) -> i128 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    }
}

/// What a file's content hash is remembered under.
#[derive(PartialEq, Eq, Hash)]
struct FileId {
    path: PathBuf,
    stamp: Stamp,
}

/// Size, change times (Unix-epoch ns) and device/inode of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
struct Stamp {
    len: u64,
    times: [i128; 2],
    node: [u64; 2],
}

/// `verified.json`: the stat a cached artifact had when its sha256 was checked.
#[derive(serde::Serialize, serde::Deserialize)]
struct Verified {
    sha256: String,
    stamp: Stamp,
}

fn file_id(path: &Path) -> Result<FileId> {
    let m = std::fs::metadata(path).map_err(io_err(path))?;
    #[cfg(unix)]
    let (times, node) = {
        use std::os::unix::fs::MetadataExt;
        let ns = |s: i64, n: i64| i128::from(s) * 1_000_000_000 + i128::from(n);
        ([ns(m.mtime(), m.mtime_nsec()), ns(m.ctime(), m.ctime_nsec())], [m.dev(), m.ino()])
    };
    #[cfg(windows)]
    let (times, node) = {
        use std::os::windows::fs::MetadataExt;
        // 100 ns ticks since 1601.
        let ns = |t: u64| (i128::from(t) - 116_444_736_000_000_000) * 100;
        ([ns(m.last_write_time()), ns(m.creation_time())], [0, 0])
    };
    #[cfg(not(any(unix, windows)))]
    let (times, node) = {
        // Unknown times are never settled.
        ([m.modified().map_or(i128::MAX / 2, unix_ns), 0], [0, 0])
    };
    Ok(FileId { path: path.to_owned(), stamp: Stamp { len: m.len(), times, node } })
}

fn hash_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).map_err(io_err(path))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(io_err(path))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Verify `path` against an expected sha256 (hex, case-insensitive).
pub fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(PluginError::Integrity { path: path.to_owned(), expected: expected.to_owned(), actual })
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_lookup_and_tamper_detection() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("c")).unwrap();
        let src = dir.path().join("lib.so");
        std::fs::write(&src, b"fake library").unwrap();
        let key = sha256_hex(b"k");
        assert!(cache.lookup(&key).unwrap().is_none());

        let entry = CacheEntry {
            key: key.clone(),
            id: "t".into(),
            file: String::new(),
            sha256: String::new(),
            size: 0,
            source: "test".into(),
            target: "native".into(),
            built_at_unix: 0,
        };
        let stored = cache.store(&key, &src, entry).unwrap();
        assert!(!stored.cached);
        let hit = cache.lookup(&key).unwrap().unwrap();
        assert!(hit.cached);
        assert_eq!(hit.sha256, stored.sha256);

        std::fs::write(&hit.path, b"tampered").unwrap();
        assert!(cache.lookup(&key).unwrap().is_none(), "tampered entry must be evicted");
    }

    #[test]
    fn lookups_trust_the_recorded_stat_and_rehash_when_it_moves() {
        let dir = tempfile::tempdir().unwrap();
        let key = sha256_hex(b"v");
        let (cache, a) = stored(dir.path(), &key, "lib.so", b"abc");
        let verified = a.path.with_file_name("verified.json");
        cache.lookup(&key).unwrap().unwrap();
        assert!(!verified.exists(), "a file written moments ago is not recorded as verified");

        // A record for the current stat is trusted without reading the file:
        // swap the content behind it while keeping the stat.
        std::fs::write(&a.path, b"abd").unwrap();
        let stamp = file_id(&a.path).unwrap().stamp;
        let record = |sha256: &str| serde_json::to_vec(&Verified { sha256: sha256.into(), stamp }).unwrap();
        std::fs::write(&verified, record(ABC)).unwrap();
        assert_eq!(cache.lookup(&key).unwrap().unwrap().sha256, ABC);
        // A record for another hash, or another stat, is not.
        std::fs::write(&verified, record(&sha256_hex(b"abd"))).unwrap();
        assert!(cache.lookup(&key).unwrap().is_none(), "content no longer matches the entry");

        let (cache, a) = stored(dir.path(), &key, "lib.so", b"abc");
        std::fs::write(a.path.with_file_name("verified.json"), record(ABC)).unwrap();
        std::fs::write(&a.path, b"xyz!").unwrap();
        assert!(cache.lookup(&key).unwrap().is_none(), "a moved stat means a rehash");
    }

    #[test]
    fn verify_sha256_reports_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a");
        std::fs::write(&f, b"abc").unwrap();
        verify_sha256(&f, "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD").unwrap();
        assert!(matches!(verify_sha256(&f, "00"), Err(PluginError::Integrity { .. })));
    }

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn entry(key: &str) -> CacheEntry {
        CacheEntry {
            key: key.into(),
            id: "t".into(),
            file: "ignored".into(),
            sha256: "ignored".into(),
            size: 0,
            source: "test".into(),
            target: "host".into(),
            built_at_unix: 7,
        }
    }

    fn stored(dir: &Path, key: &str, name: &str, data: &[u8]) -> (Cache, Artifact) {
        let cache = Cache::new(dir.join("c")).unwrap();
        let src = dir.join(name);
        std::fs::write(&src, data).unwrap();
        let a = cache.store(key, &src, entry(key)).unwrap();
        (cache, a)
    }

    #[test]
    fn sha256_helpers_produce_lowercase_hex() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(sha256_file(&empty).unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), ABC);
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        // Larger than the read buffer, so several chunks are hashed.
        let big = dir.path().join("big");
        let data = vec![7u8; (1 << 16) * 3 + 5];
        std::fs::write(&big, &data).unwrap();
        assert_eq!(sha256_file(&big).unwrap(), sha256_hex(&data));
    }

    #[test]
    fn remembered_hashes_follow_every_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(sha256_file(&f).unwrap(), ABC);
        assert_eq!(sha256_file(&f).unwrap(), ABC);
        // Same length, rewritten in place within one timestamp tick (Linux
        // keeps mtime and ctime equal): a fresh file is never remembered.
        std::fs::write(&f, b"abd").unwrap();
        assert_ne!(sha256_file(&f).unwrap(), ABC);
        // Replaced by another file (new inode) with the old content and mtime.
        let g = dir.path().join("b");
        std::fs::write(&g, b"xyz").unwrap();
        std::fs::rename(&g, &f).unwrap();
        assert_eq!(sha256_file(&f).unwrap(), sha256_hex(b"xyz"));
    }

    #[test]
    fn only_files_older_than_their_timestamp_granularity_are_remembered() {
        let id = |mtime: i128, ctime: i128| Stamp { len: 0, times: [mtime, ctime], node: [0, 0] };
        let s = SETTLED.as_nanos() as i128;
        let now = 1_000 * s;
        assert!(settled(&id(now - s, now - 2 * s), now));
        assert!(!settled(&id(now - 2 * s, now - s + 1), now), "a recent ctime alone is enough");
        assert!(!settled(&id(now, now), now));
        assert!(!settled(&id(now + s, now - s), now), "times from the future");
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a");
        std::fs::write(&f, b"abc").unwrap();
        let fresh = file_id(&f).unwrap().stamp;
        let now = unix_ns(std::time::SystemTime::now());
        assert!(!settled(&fresh, now) && settled(&fresh, now + s + 1_000_000_000), "{:?}", fresh.times);
    }

    #[test]
    fn verify_sha256_fails_on_missing_file_and_on_any_other_hash() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a");
        assert!(matches!(verify_sha256(&f, ABC), Err(PluginError::Io { path, .. }) if path == f));
        std::fs::write(&f, b"abc").unwrap();
        verify_sha256(&f, ABC).unwrap();
        let truncated = &ABC[..63];
        let mut flipped = ABC.to_owned();
        flipped.replace_range(..1, "c");
        for expected in ["", truncated, flipped.as_str(), " ba7816bf", "sha256:ba7816bf"] {
            match verify_sha256(&f, expected) {
                Err(PluginError::Integrity { path, expected: e, actual }) => {
                    assert_eq!((path, e.as_str(), actual.as_str()), (f.clone(), expected, ABC));
                }
                other => panic!("{expected:?}: expected Integrity, got {other:?}"),
            }
        }
    }

    #[test]
    fn store_records_metadata_and_leaves_no_temp_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let key = sha256_hex(b"meta");
        let (cache, a) = stored(dir.path(), &key, "libx.dylib", b"0123456789");
        assert_eq!(a.path, cache.root().join("artifacts").join(&key[..2]).join(&key).join("plugin.dylib"));
        assert_eq!((a.sha256.as_str(), a.key.as_deref()), (sha256_hex(b"0123456789").as_str(), Some(key.as_str())));

        let meta: CacheEntry =
            serde_json::from_slice(&std::fs::read(a.path.with_file_name("entry.json")).unwrap()).unwrap();
        assert_eq!((meta.file.as_str(), meta.size, meta.built_at_unix), ("plugin.dylib", 10, 7));
        assert_eq!(meta.sha256, a.sha256, "the hash is of the stored copy, not the caller's value");

        let names: Vec<_> = std::fs::read_dir(cache.root().join("artifacts"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [&key[..2]], "no `.tmp-*` directory survives a store");
    }

    #[test]
    fn store_without_extension_uses_bin_and_replaces_an_existing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let key = sha256_hex(b"replace");
        let (cache, first) = stored(dir.path(), &key, "first.so", b"one");
        let src = dir.path().join("noext");
        std::fs::write(&src, b"two").unwrap();
        let second = cache.store(&key, &src, entry(&key)).unwrap();
        assert!(second.path.ends_with("plugin.bin"));
        assert!(!first.path.exists(), "the old entry is replaced as a whole");
        let hit = cache.lookup(&key).unwrap().unwrap();
        assert_eq!((hit.path, hit.sha256), (second.path, sha256_hex(b"two")));
    }

    #[test]
    fn store_of_a_missing_file_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path()).unwrap();
        let key = sha256_hex(b"missing");
        assert!(matches!(cache.store(&key, &dir.path().join("nope.so"), entry(&key)), Err(PluginError::Io { .. })));
        assert!(cache.lookup(&key).unwrap().is_none());
    }

    #[test]
    fn corrupt_or_incomplete_entries_are_evicted_as_misses() {
        let dir = tempfile::tempdir().unwrap();
        let key = sha256_hex(b"corrupt");
        let (cache, a) = stored(dir.path(), &key, "lib.so", b"data");
        let entry_dir = a.path.parent().unwrap().to_owned();
        std::fs::write(entry_dir.join("entry.json"), b"{ not json").unwrap();
        assert!(cache.lookup(&key).unwrap().is_none());
        assert!(!entry_dir.exists(), "unparsable metadata evicts the entry");

        let key = sha256_hex(b"no artifact");
        let a = cache.store(&key, &dir.path().join("lib.so"), entry(&key)).unwrap();
        std::fs::remove_file(&a.path).unwrap();
        assert!(cache.lookup(&key).unwrap().is_none());
        assert!(!a.path.parent().unwrap().exists(), "a missing artifact evicts the entry");
    }

    #[test]
    fn gc_keeps_only_listed_keys() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("c")).unwrap();
        assert_eq!(cache.gc(&HashSet::new()).unwrap(), 0, "an empty cache has nothing to collect");
        let src = dir.path().join("lib.so");
        std::fs::write(&src, b"x").unwrap();
        let keys: Vec<String> = (0..4).map(|i| sha256_hex(&[i])).collect();
        for k in &keys {
            cache.store(k, &src, entry(k)).unwrap();
        }
        let keep: HashSet<String> = keys[..2].iter().cloned().collect();
        assert_eq!(cache.gc(&keep).unwrap(), 2);
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(cache.lookup(k).unwrap().is_some(), i < 2, "key {i}");
        }
        assert_eq!(cache.gc(&keep).unwrap(), 0, "gc is idempotent");
    }

    #[test]
    fn cache_lock_is_exclusive_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path()).unwrap();
        let other = || File::options().write(true).open(dir.path().join(".lock")).unwrap();
        let held = cache.lock().unwrap();
        assert!(other().try_lock().is_err(), "a second lock must wait");
        drop(held);
        other().try_lock().unwrap();
    }

    #[test]
    fn new_creates_the_layout_and_reports_unusable_roots() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("a/b")).unwrap();
        for d in ["artifacts", "target", "git", "loaded", "meta"] {
            assert!(cache.root().join(d).is_dir(), "{d}");
        }
        assert_eq!(cache.target_dir(), cache.root().join("target"));
        assert_eq!(cache.git_dir(), cache.root().join("git"));
        assert_eq!(cache.shadow_dir(), cache.root().join("loaded"));
        assert_eq!(cache.compiled_dir(), cache.root().join("compiled"));
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert!(matches!(Cache::new(&file), Err(PluginError::Io { .. })));
    }
}
