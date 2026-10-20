//! Watching uses a scripted backend, so it runs without d2.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use d2_render::watch::{WatchConfig, WatchEvent, output_path, watch_channel};
use d2_render::{Backend, BackendOutput, Format, RenderOptions, Renderer, Result};

/// Copies the source into the output, wrapped as a fake SVG.
#[derive(Debug)]
struct EchoBackend;

impl Backend for EchoBackend {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn supports(&self, f: Format) -> bool {
        f == Format::Svg
    }
    fn render(
        &self,
        input: &Path,
        output: &Path,
        _: Format,
        _: &RenderOptions,
    ) -> Result<BackendOutput> {
        let src = fs::read_to_string(input)?;
        fs::write(output, format!("<svg><!-- {src} --></svg>"))?;
        Ok(BackendOutput::default())
    }
    fn validate(&self, _: &Path) -> Result<()> {
        Ok(())
    }
    fn version(&self) -> Result<String> {
        Ok("echo".into())
    }
}

fn next_render(rx: &Receiver<WatchEvent>, want: &str) -> (PathBuf, PathBuf) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Ok(WatchEvent::Rendered {
            input,
            output,
            result,
        }) = rx.recv_timeout(Duration::from_millis(200))
        {
            result.expect("render failed");
            if fs::read_to_string(&output)
                .unwrap_or_default()
                .contains(want)
            {
                return (input, output);
            }
        }
    }
    panic!("no render containing {want:?} within 20s");
}

#[test]
fn rerenders_on_change() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let src = root.join("a.d2");
    fs::write(&src, "v1").unwrap();
    let config = WatchConfig {
        debounce: Duration::from_millis(100),
        render_on_start: true,
        ..WatchConfig::default()
    };
    let (handle, rx) = watch_channel(
        Renderer::with_backend(EchoBackend),
        std::slice::from_ref(&root),
        config,
    )
    .unwrap();
    let (input, output) = next_render(&rx, "v1");
    assert_eq!(input, src);
    assert_eq!(output, root.join("a.svg"));

    // Give the OS watcher a moment to settle before changing the file.
    std::thread::sleep(Duration::from_millis(300));
    fs::write(&src, "v2").unwrap();
    next_render(&rx, "v2");

    // New files in the watched tree are picked up too.
    fs::create_dir(root.join("sub")).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    fs::write(root.join("sub/b.d2"), "v3").unwrap();
    let (input, output) = next_render(&rx, "v3");
    assert_eq!(input, root.join("sub/b.d2"));
    assert_eq!(output, root.join("sub/b.svg"));
    handle.stop();
}

#[test]
fn watches_single_file_into_out_dir() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let src = root.join("one.d2");
    let other = root.join("other.d2");
    fs::write(&src, "x").unwrap();
    let out_dir = root.join("out");
    let config = WatchConfig {
        out_dir: Some(out_dir.clone()),
        debounce: Duration::from_millis(100),
        ..WatchConfig::default()
    };
    let (_handle, rx) = watch_channel(
        Renderer::with_backend(EchoBackend),
        std::slice::from_ref(&src),
        config,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    fs::write(&other, "ignored").unwrap();
    fs::write(&src, "changed").unwrap();
    let (input, output) = next_render(&rx, "changed");
    assert_eq!(input, src);
    assert_eq!(output, out_dir.join("one.svg"));
    assert!(!out_dir.join("other.svg").exists());
}

#[test]
fn output_mapping() {
    let c = WatchConfig {
        out_dir: Some(PathBuf::from("/o")),
        format: Format::Png,
        ..WatchConfig::default()
    };
    assert_eq!(
        output_path(&c, Some(Path::new("/r")), Path::new("/r/a/b.d2")),
        PathBuf::from("/o/a/b.png")
    );
    assert_eq!(
        output_path(&c, None, Path::new("/r/a/b.d2")),
        PathBuf::from("/o/b.png")
    );
    let c = WatchConfig::default();
    assert_eq!(
        output_path(&c, None, Path::new("/r/b.d2")),
        PathBuf::from("/r/b.svg")
    );
}

#[cfg(feature = "native")]
#[test]
fn native_watch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let src = root.join("n.d2");
    fs::write(&src, "a -> b").unwrap();
    let config = WatchConfig {
        debounce: Duration::from_millis(100),
        render_on_start: true,
        ..WatchConfig::default()
    };
    let (_h, rx) = watch_channel(Renderer::new(), std::slice::from_ref(&root), config).unwrap();
    next_render(&rx, "<svg");
    std::thread::sleep(Duration::from_millis(300));
    fs::write(&src, "a -> b -> natively").unwrap();
    next_render(&rx, "natively");
}

#[cfg(feature = "cli")]
#[test]
fn real_d2_watch() {
    let r = Renderer::cli();
    if r.version().is_err() {
        eprintln!("SKIP real_d2_watch: d2 not available");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let src = root.join("w.d2");
    fs::write(&src, "a -> b").unwrap();
    let config = WatchConfig {
        debounce: Duration::from_millis(100),
        ..WatchConfig::default()
    };
    let (_h, rx) = watch_channel(r, std::slice::from_ref(&root), config).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    fs::write(&src, "a -> b -> watched").unwrap();
    next_render(&rx, "watched");
}
