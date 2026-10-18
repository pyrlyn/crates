//! Re-render `.d2` files when they change (feature `watch`).
//!
//! Uses [`notify`] with [`notify_debouncer_mini`], so a burst of writes from
//! an editor triggers one render. A watched file is observed through its
//! parent directory, which survives editors that save by renaming.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};

use crate::{Error, Format, RenderReport, Renderer, Result};

/// How to watch.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Output format; outputs get its extension.
    pub format: Format,
    /// Write outputs here (mirroring the layout under each watched
    /// directory) instead of next to their sources.
    pub out_dir: Option<PathBuf>,
    /// Quiet period before a change is rendered.
    pub debounce: Duration,
    /// Render every watched source once at start-up.
    pub render_on_start: bool,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            format: Format::Svg,
            out_dir: None,
            debounce: Duration::from_millis(200),
            render_on_start: false,
        }
    }
}

/// Something the watcher did.
// Events are moved straight to the callback, so the size gap is harmless.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum WatchEvent {
    /// A source was rendered (successfully or not).
    Rendered {
        /// The `.d2` file.
        input: PathBuf,
        /// Where it was written.
        output: PathBuf,
        /// The render result.
        result: Result<RenderReport>,
    },
    /// The file watcher itself reported an error.
    WatchError(Error),
}

/// Keeps the watcher alive; dropping it (or [`WatchHandle::stop`]) stops it.
pub struct WatchHandle {
    debouncer: Option<Debouncer<RecommendedWatcher>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchHandle")
            .field("running", &self.debouncer.is_some())
            .finish()
    }
}

impl WatchHandle {
    /// Stop watching and wait for the worker thread.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        // Dropping the debouncer drops its sender, which ends the worker loop.
        self.debouncer.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone)]
struct Root {
    path: PathBuf,
    is_dir: bool,
}

/// Watch `paths` (files or directories) and call `on_event` for every
/// render. Returns once the watcher is running.
pub fn watch<F>(
    renderer: Renderer,
    paths: &[PathBuf],
    config: WatchConfig,
    mut on_event: F,
) -> Result<WatchHandle>
where
    F: FnMut(WatchEvent) + Send + 'static,
{
    let mut roots = Vec::new();
    for p in paths {
        let path = p.canonicalize()?;
        let is_dir = path.is_dir();
        roots.push(Root { path, is_dir });
    }
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer =
        new_debouncer(config.debounce, tx).map_err(|e| Error::Watch(e.to_string()))?;
    for r in &roots {
        let (target, mode) = if r.is_dir {
            (r.path.clone(), RecursiveMode::Recursive)
        } else {
            let parent = r.path.parent().unwrap_or(Path::new(".")).to_path_buf();
            (parent, RecursiveMode::NonRecursive)
        };
        debouncer
            .watcher()
            .watch(&target, mode)
            .map_err(|e| Error::Watch(e.to_string()))?;
    }

    let initial: Vec<PathBuf> = if config.render_on_start {
        let mut v = BTreeSet::new();
        for r in &roots {
            if r.is_dir {
                v.extend(crate::fresh::find_sources(&r.path)?);
            } else {
                v.insert(r.path.clone());
            }
        }
        v.into_iter().collect()
    } else {
        Vec::new()
    };

    let thread = std::thread::Builder::new()
        .name("d2-render-watch".into())
        .spawn(move || {
            for input in initial {
                on_event(render_one(&renderer, &roots, &config, input));
            }
            while let Ok(batch) = rx.recv() {
                match batch {
                    Ok(events) => {
                        let changed: BTreeSet<PathBuf> = events
                            .into_iter()
                            .map(|e| e.path)
                            .filter(|p| p.extension().is_some_and(|e| e == "d2"))
                            .filter_map(|p| p.canonicalize().ok())
                            .filter(|p| p.is_file() && owning_root(&roots, p).is_some())
                            .collect();
                        for input in changed {
                            on_event(render_one(&renderer, &roots, &config, input));
                        }
                    }
                    Err(e) => on_event(WatchEvent::WatchError(Error::Watch(format!("{e:?}")))),
                }
            }
        })?;

    Ok(WatchHandle {
        debouncer: Some(debouncer),
        thread: Some(thread),
    })
}

/// [`watch`] delivering events on a channel instead of a callback.
pub fn watch_channel(
    renderer: Renderer,
    paths: &[PathBuf],
    config: WatchConfig,
) -> Result<(WatchHandle, Receiver<WatchEvent>)> {
    let (tx, rx) = mpsc::channel();
    let handle = watch(renderer, paths, config, move |ev| {
        let _ = tx.send(ev);
    })?;
    Ok((handle, rx))
}

fn owning_root<'a>(roots: &'a [Root], p: &Path) -> Option<&'a Root> {
    roots.iter().find(|r| {
        if r.is_dir {
            p.starts_with(&r.path)
        } else {
            p == r.path
        }
    })
}

/// Where `input` renders to under `config`.
pub fn output_path(config: &WatchConfig, root: Option<&Path>, input: &Path) -> PathBuf {
    let ext = config.format.extension();
    match (&config.out_dir, root) {
        (Some(out), Some(root)) => {
            let rel = input
                .strip_prefix(root)
                .ok()
                .filter(|r| !r.as_os_str().is_empty())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(input.file_name().unwrap_or_default()));
            out.join(rel).with_extension(ext)
        }
        (Some(out), None) => out
            .join(input.file_name().unwrap_or_default())
            .with_extension(ext),
        (None, _) => input.with_extension(ext),
    }
}

fn render_one(
    renderer: &Renderer,
    roots: &[Root],
    config: &WatchConfig,
    input: PathBuf,
) -> WatchEvent {
    let root = owning_root(roots, &input)
        .filter(|r| r.is_dir)
        .map(|r| r.path.as_path());
    let output = output_path(config, root, &input);
    let result = renderer.render_file_report(&input, &output, config.format);
    WatchEvent::Rendered {
        input,
        output,
        result,
    }
}
