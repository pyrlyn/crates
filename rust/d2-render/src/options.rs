//! Output formats and render options.
//!
//! Every option maps to a flag listed by `d2 --help` (d2 v0.9); nothing here
//! is passed to d2 unless it is set.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;

use crate::{Error, Result};

/// An output format d2 can write. d2 picks the format from the output file
/// extension; [`Format::extension`] is that extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// Scalable vector graphics (the default).
    Svg,
    /// Raster image.
    Png,
    /// PDF document.
    Pdf,
    /// PowerPoint deck.
    Pptx,
    /// Animated GIF (multi-board diagrams).
    Gif,
    /// ASCII/Unicode art.
    Txt,
}

impl Format {
    /// All formats, in declaration order.
    pub const ALL: [Format; 6] = [
        Format::Svg,
        Format::Png,
        Format::Pdf,
        Format::Pptx,
        Format::Gif,
        Format::Txt,
    ];

    /// File extension without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Svg => "svg",
            Format::Png => "png",
            Format::Pdf => "pdf",
            Format::Pptx => "pptx",
            Format::Gif => "gif",
            Format::Txt => "txt",
        }
    }

    /// MIME type, e.g. for data URIs.
    pub fn mime(self) -> &'static str {
        match self {
            Format::Svg => "image/svg+xml",
            Format::Png => "image/png",
            Format::Pdf => "application/pdf",
            Format::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Format::Gif => "image/gif",
            Format::Txt => "text/plain",
        }
    }

    /// Format for an extension (case-insensitive, without the dot).
    pub fn from_extension(ext: &str) -> Option<Format> {
        Format::ALL
            .into_iter()
            .find(|f| f.extension().eq_ignore_ascii_case(ext))
    }

    /// Format inferred from a path's extension.
    pub fn from_path(path: &Path) -> Result<Format> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Format::from_extension)
            .ok_or_else(|| Error::UnknownFormat(path.to_path_buf()))
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// A layout engine (`--layout`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Layout {
    /// Dagre, d2's default.
    Dagre,
    /// Eclipse Layout Kernel.
    Elk,
    /// TALA (bundled with recent d2 releases).
    Tala,
    /// Any other engine name, e.g. a plugin on `$PATH`.
    Other(String),
}

impl Layout {
    /// The name passed to `--layout`.
    pub fn as_str(&self) -> &str {
        match self {
            Layout::Dagre => "dagre",
            Layout::Elk => "elk",
            Layout::Tala => "tala",
            Layout::Other(s) => s,
        }
    }
}

impl fmt::Display for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Options for one render. `Default` passes no flags, so d2 uses its own
/// defaults (and honours its environment variables such as `D2_THEME`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderOptions {
    /// `--theme`: theme id (`d2 themes` lists them).
    pub theme: Option<i64>,
    /// `--dark-theme`: theme used when the viewer prefers dark mode.
    pub dark_theme: Option<i64>,
    /// `--layout`.
    pub layout: Option<Layout>,
    /// `--pad`: padding in pixels around the diagram.
    pub pad: Option<u32>,
    /// `--sketch`: hand-drawn look.
    pub sketch: bool,
    /// `--center`: center the SVG in its viewbox.
    pub center: bool,
    /// `--scale`.
    pub scale: Option<f64>,
    /// `--timeout`: seconds before d2 gives up.
    pub timeout_secs: Option<u32>,
    /// `--target`: board to render.
    pub target: Option<String>,
    /// `--no-xml-tag`: omit `<?xml ...?>` from SVG output.
    pub no_xml_tag: bool,
    /// `--omit-version`: omit the d2 version from the output.
    pub omit_version: bool,
    /// `--salt`: suffix for SVG ids, for several diagrams in one page.
    pub salt: Option<String>,
    /// `--animate-interval` in milliseconds (multi-board SVG/GIF).
    pub animate_interval_ms: Option<u32>,
    /// Record a source hash in the output (SVG comment) or in a
    /// `<output>.d2hash` sidecar, so staleness can be checked later.
    pub stamp: bool,
}

impl RenderOptions {
    /// Same as `Default::default()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set `--theme`.
    pub fn theme(mut self, id: i64) -> Self {
        self.theme = Some(id);
        self
    }

    /// Set `--dark-theme`.
    pub fn dark_theme(mut self, id: i64) -> Self {
        self.dark_theme = Some(id);
        self
    }

    /// Set `--layout`.
    pub fn layout(mut self, layout: Layout) -> Self {
        self.layout = Some(layout);
        self
    }

    /// Set `--pad`.
    pub fn pad(mut self, px: u32) -> Self {
        self.pad = Some(px);
        self
    }

    /// Set `--sketch`.
    pub fn sketch(mut self, on: bool) -> Self {
        self.sketch = on;
        self
    }

    /// Set `--center`.
    pub fn center(mut self, on: bool) -> Self {
        self.center = on;
        self
    }

    /// Set `--scale`.
    pub fn scale(mut self, scale: f64) -> Self {
        self.scale = Some(scale);
        self
    }

    /// Set `--timeout` in seconds.
    pub fn timeout_secs(mut self, secs: u32) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Set `--target`.
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// Set `--no-xml-tag`.
    pub fn no_xml_tag(mut self, on: bool) -> Self {
        self.no_xml_tag = on;
        self
    }

    /// Set `--omit-version`.
    pub fn omit_version(mut self, on: bool) -> Self {
        self.omit_version = on;
        self
    }

    /// Set `--salt`.
    pub fn salt(mut self, salt: impl Into<String>) -> Self {
        self.salt = Some(salt.into());
        self
    }

    /// Set `--animate-interval` in milliseconds.
    pub fn animate_interval_ms(mut self, ms: u32) -> Self {
        self.animate_interval_ms = Some(ms);
        self
    }

    /// Record a source hash for [`crate::Renderer::is_stale`].
    pub fn stamp(mut self, on: bool) -> Self {
        self.stamp = on;
        self
    }

    /// The d2 flags these options translate to, in a stable order.
    pub fn to_args(&self) -> Vec<OsString> {
        let mut a: Vec<OsString> = Vec::new();
        if let Some(t) = self.theme {
            a.push(format!("--theme={t}").into());
        }
        if let Some(t) = self.dark_theme {
            a.push(format!("--dark-theme={t}").into());
        }
        if let Some(l) = &self.layout {
            a.push(format!("--layout={l}").into());
        }
        if let Some(p) = self.pad {
            a.push(format!("--pad={p}").into());
        }
        if self.sketch {
            a.push("--sketch".into());
        }
        if self.center {
            a.push("--center".into());
        }
        if let Some(s) = self.scale {
            a.push(format!("--scale={s}").into());
        }
        if let Some(t) = self.timeout_secs {
            a.push(format!("--timeout={t}").into());
        }
        if let Some(t) = &self.target {
            a.push(format!("--target={t}").into());
        }
        if self.no_xml_tag {
            a.push("--no-xml-tag".into());
        }
        if self.omit_version {
            a.push("--omit-version".into());
        }
        if let Some(s) = &self.salt {
            a.push(format!("--salt={s}").into());
        }
        if let Some(ms) = self.animate_interval_ms {
            a.push(format!("--animate-interval={ms}").into());
        }
        a
    }

    /// A stable string describing everything that changes the output; it is
    /// part of the freshness hash, so changing an option makes outputs stale.
    pub fn fingerprint(&self) -> String {
        let args: Vec<String> = self
            .to_args()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        args.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_from_path() {
        assert_eq!(
            Format::from_path(Path::new("a/b.SVG")).unwrap(),
            Format::Svg
        );
        assert_eq!(Format::from_path(Path::new("b.png")).unwrap(), Format::Png);
        assert!(Format::from_path(Path::new("b")).is_err());
        assert!(Format::from_path(Path::new("b.xyz")).is_err());
    }

    #[test]
    fn args() {
        let o = RenderOptions::new()
            .theme(200)
            .layout(Layout::Elk)
            .pad(10)
            .sketch(true);
        let args: Vec<String> = o
            .to_args()
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(
            args,
            ["--theme=200", "--layout=elk", "--pad=10", "--sketch"]
        );
        assert!(RenderOptions::default().to_args().is_empty());
    }
}
