//! Opt-in file watcher: [`PluginManager::watch`] → [`Watcher`] → reload.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::error::{PluginError, Result, io_err};
use crate::manager::PluginManager;
use crate::manifest::Source;

/// Watcher settings.
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// Quiet time after the last event on a path before it counts as one
    /// change: an editor's save is several writes and renames.
    pub debounce: Duration,
}

impl Default for WatchOptions {
    fn default() -> Self {
        WatchOptions { debounce: Duration::from_millis(300) }
    }
}

/// One watched location and the plugin it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Watch {
    id: String,
    /// The artifact file, or the crate directory when `recursive`.
    root: PathBuf,
    recursive: bool,
    /// Never trigger a reload: the reload itself writes there and would loop.
    ignore: Vec<PathBuf>,
}

impl Watch {
    /// What to watch for a plugin, or `None` when nothing local can change
    /// (git sources are fetched into the cache). A crate's path dependencies
    /// outside its directory are not watched.
    pub(crate) fn for_source(id: &str, source: &Source, resolved: &Path, ignore: &[PathBuf]) -> Result<Option<Self>> {
        let (root, recursive) = match source {
            Source::Artifact(_) => {
                // The OS watcher reports canonical paths (`/private/var` on macOS).
                let parent = resolved.parent().unwrap_or(Path::new("."));
                let dir = parent.canonicalize().map_err(io_err(parent))?;
                (dir.join(resolved.file_name().unwrap_or_default()), false)
            }
            Source::Path(_) => (resolved.canonicalize().map_err(io_err(resolved))?, true),
            Source::Git(_) => return Ok(None),
        };
        let ignore = ignore.iter().map(|p| p.canonicalize().unwrap_or_else(|_| p.clone())).collect();
        Ok(Some(Watch { id: id.to_owned(), root, recursive, ignore }))
    }

    fn matches(&self, path: &Path, kind: &EventKind) -> bool {
        if self.ignore.iter().any(|i| path.starts_with(i)) {
            return false;
        }
        if !self.recursive {
            return path == self.root;
        }
        let Ok(rel) = path.strip_prefix(&self.root) else { return false };
        // An event on the crate directory itself (its own metadata, or on
        // macOS a late FSEvents record of its creation) changes no file in
        // it; the files report their own events. Only its removal counts.
        if rel.as_os_str().is_empty() {
            return matches!(kind, EventKind::Remove(_));
        }
        !rel.components().any(|c| matches!(c, Component::Normal(n) if n == "target" || n == ".git"))
    }

    /// The directory handed to the OS watcher. An artifact is watched through
    /// its directory because editors and linkers replace the file.
    fn dir(&self) -> &Path {
        if self.recursive { &self.root } else { self.root.parent().unwrap_or(&self.root) }
    }
}

enum Msg {
    Changed(BTreeSet<String>, Instant),
    Stop,
}

/// Reads never change a plugin, and inotify reports every open: counting them
/// would make each reload (which reads the files) trigger the next one.
fn is_change(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(_))
}

/// Collects the ids of plugins whose files changed. Created by
/// [`PluginManager::watch`]; nothing is watched until then. Dropping it (or
/// [`Watcher::stop`]) stops watching.
pub struct Watcher {
    _watcher: RecommendedWatcher,
    debounce: Duration,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// Plugin id → time of its latest change.
    pending: BTreeMap<String, Instant>,
    /// Set by [`Reloader`]; only its thread reads it.
    stopped: bool,
}

impl Watcher {
    pub(crate) fn new(watches: Vec<Watch>, opts: &WatchOptions) -> Result<Self> {
        let mut dirs: BTreeMap<PathBuf, bool> = BTreeMap::new();
        for w in &watches {
            *dirs.entry(w.dir().to_owned()).or_default() |= w.recursive;
        }
        let (tx, rx) = mpsc::channel();
        let sink = tx.clone();
        let handler = move |res: notify::Result<Event>| match res {
            Ok(event) if is_change(&event.kind) => {
                let ids: BTreeSet<String> = event
                    .paths
                    .iter()
                    .flat_map(|p| watches.iter().filter(|w| w.matches(p, &event.kind)).map(|w| w.id.clone()))
                    .collect();
                if !ids.is_empty() {
                    tracing::debug!(?ids, kind = ?event.kind, "plugin files changed");
                    let _ = sink.send(Msg::Changed(ids, Instant::now()));
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "file watcher error"),
        };
        let mut watcher =
            RecommendedWatcher::new(handler, notify::Config::default()).map_err(watch_err(Path::new(".")))?;
        for (dir, recursive) in dirs {
            let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            watcher.watch(&dir, mode).map_err(watch_err(&dir))?;
        }
        Ok(Watcher { _watcher: watcher, debounce: opts.debounce, tx, rx, pending: BTreeMap::new(), stopped: false })
    }

    /// Block until a change has settled (no new event on that plugin for the
    /// debounce time) or `timeout` passes; true when one has.
    pub fn wait(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        loop {
            while let Ok(m) = self.rx.try_recv() {
                self.absorb(m);
            }
            let now = Instant::now();
            if self.stopped {
                return false;
            }
            if self.pending.values().any(|t| now.duration_since(*t) >= self.debounce) {
                return true;
            }
            let settles = self.pending.values().min().map(|t| *t + self.debounce);
            let until = match (deadline, settles) {
                (Some(d), Some(s)) => d.min(s),
                (d, s) => d.or(s).unwrap_or(now + Duration::from_secs(3600)),
            };
            if deadline.is_some_and(|d| now >= d) {
                return false;
            }
            if let Ok(m) = self.rx.recv_timeout(until.saturating_duration_since(now)) {
                self.absorb(m);
            }
        }
    }

    /// Take the ids whose changes have settled, without waiting.
    pub fn take(&mut self) -> BTreeSet<String> {
        self.wait(Duration::ZERO);
        let now = Instant::now();
        let (ready, busy) =
            std::mem::take(&mut self.pending).into_iter().partition(|(_, t)| now.duration_since(*t) >= self.debounce);
        self.pending = busy;
        ready.into_keys().collect::<BTreeSet<_>>()
    }

    /// Stop watching; changes not yet taken are dropped.
    pub fn stop(self) {}

    fn absorb(&mut self, msg: Msg) {
        match msg {
            Msg::Changed(ids, at) => self.pending.extend(ids.into_iter().map(|id| (id, at))),
            Msg::Stop => self.stopped = true,
        }
    }

    /// Reload changed plugins on a background thread, reporting every swap or
    /// failure to `report`. A failure keeps the old instance serving.
    pub fn spawn<F>(mut self, manager: Arc<Mutex<PluginManager>>, mut report: F) -> Result<Reloader>
    where
        F: FnMut(&str, Result<()>) + Send + 'static,
    {
        let tx = self.tx.clone();
        let run = move || {
            while !self.stopped {
                if !self.wait(Duration::from_secs(3600)) {
                    continue;
                }
                let Ok(mut m) = manager.lock() else {
                    tracing::error!("plugin manager mutex poisoned; watcher stops");
                    return;
                };
                for (id, r) in m.reload_changed(&mut self) {
                    report(&id, r);
                }
            }
        };
        let thread = std::thread::Builder::new()
            .name("plugwright-watch".into())
            .spawn(run)
            .map_err(io_err(Path::new("plugwright-watch")))?;
        Ok(Reloader { tx, thread: Some(thread) })
    }
}

/// The background reload thread from [`Watcher::spawn`]. Stopping (or
/// dropping) it joins the thread, so never do that while holding the
/// manager's lock.
pub struct Reloader {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl Reloader {
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("watcher thread panicked");
        }
    }
}

impl Drop for Reloader {
    fn drop(&mut self) {
        self.halt();
    }
}

fn watch_err(path: &Path) -> impl FnOnce(notify::Error) -> PluginError + '_ {
    move |e| PluginError::Io { path: path.to_owned(), source: std::io::Error::other(e) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitRef, GitSource};

    fn watch(id: &str, source: Source, resolved: &Path, ignore: &[PathBuf]) -> Watch {
        Watch::for_source(id, &source, resolved, ignore).unwrap().unwrap()
    }

    #[test]
    fn an_artifact_matches_only_its_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = dir.path().join("p.wasm");
        let w = watch("p", Source::Artifact(file.clone()), &file, &[]);
        assert_eq!(w.dir(), root, "watched through its directory");
        assert!(w.matches(&root.join("p.wasm"), &EventKind::Any));
        assert!(
            !w.matches(&root.join("p.wasm.tmp"), &EventKind::Any)
                && !w.matches(&root.join("other.wasm"), &EventKind::Any)
        );
    }

    #[test]
    fn a_crate_matches_its_sources_but_not_build_output() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let cache = dir.path().join(".plugwright");
        std::fs::create_dir(&cache).unwrap();
        let w = watch("c", Source::Path("c".into()), dir.path(), std::slice::from_ref(&cache));
        assert_eq!(w.dir(), root);
        assert!(
            w.matches(&root.join("src/lib.rs"), &EventKind::Any)
                && w.matches(&root.join("Cargo.toml"), &EventKind::Any)
        );
        for skipped in ["target/debug/libc.so", "sub/target/x", ".git/index", ".plugwright/artifacts/k"] {
            assert!(!w.matches(&root.join(skipped), &EventKind::Any), "{skipped}");
        }
        assert!(!w.matches(&root.with_file_name("elsewhere.rs"), &EventKind::Any));
        use notify::event::{ModifyKind, RemoveKind};
        assert!(!w.matches(&root, &EventKind::Modify(ModifyKind::Any)), "the directory's own events are no change");
        assert!(w.matches(&root, &EventKind::Remove(RemoveKind::Folder)), "removing it is");
    }

    #[test]
    fn git_sources_and_missing_paths() {
        let git = Source::Git(GitSource {
            url: "https://example.com/r".into(),
            reference: GitRef::DefaultBranch,
            subdir: None,
        });
        assert_eq!(Watch::for_source("g", &git, Path::new(""), &[]).unwrap(), None, "nothing local to watch");
        let gone = Path::new("/definitely/not/here");
        let err = Watch::for_source("m", &Source::Path(gone.into()), gone, &[]).unwrap_err();
        assert!(matches!(&err, PluginError::Io { path, .. } if path == gone), "{err}");
    }

    #[test]
    fn only_writes_count_as_changes() {
        use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind};
        assert!(!is_change(&EventKind::Access(AccessKind::Open(AccessMode::Read))));
        assert!(!is_change(&EventKind::Access(AccessKind::Close(AccessMode::Read))));
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Modify(ModifyKind::Any),
            EventKind::Remove(RemoveKind::File),
            EventKind::Any,
        ] {
            assert!(is_change(&kind), "{kind:?}");
        }
    }

    #[test]
    fn a_burst_of_writes_is_one_change_and_other_files_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.wasm");
        std::fs::write(&file, b"0").unwrap();
        std::fs::create_dir_all(dir.path().join("krate/target")).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let watches = vec![
            watch("p", Source::Artifact(file.clone()), &file, &[]),
            watch("k", Source::Path("krate".into()), &dir.path().join("krate"), &[]),
        ];
        let mut w = Watcher::new(watches, &WatchOptions { debounce: Duration::from_millis(150) }).unwrap();
        assert!(!w.wait(Duration::from_millis(400)), "nothing changed yet");

        // Reads are what every reload does (inotify reports each open).
        std::fs::read(&file).unwrap();
        std::fs::read_dir(dir.path().join("krate")).unwrap().for_each(drop);
        std::fs::write(dir.path().join("unrelated.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("krate/target/out"), b"x").unwrap();
        assert!(!w.wait(Duration::from_millis(600)), "{:?}", w.take());

        for n in 0..3u8 {
            std::fs::write(&file, [n]).unwrap();
        }
        assert!(w.wait(Duration::from_secs(10)));
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(w.take(), BTreeSet::from(["p".to_owned()]));
        assert!(w.take().is_empty(), "taken once");

        std::fs::write(dir.path().join("krate/lib.rs"), b"x").unwrap();
        assert!(w.wait(Duration::from_secs(10)));
        assert_eq!(w.take(), BTreeSet::from(["k".to_owned()]));
    }
}
