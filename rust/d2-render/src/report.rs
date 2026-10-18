//! The typed result of a successful render.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::diagnostic::Diagnostic;
use crate::svg::SvgInfo;
use crate::Format;

/// Everything known about one successful render.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderReport {
    /// The `.d2` file that was rendered (a temporary file for
    /// [`crate::Renderer::render_str_report`], already deleted).
    pub input: PathBuf,
    /// The written file.
    pub output: PathBuf,
    /// Its format.
    pub format: Format,
    /// Wall-clock time of the whole call.
    pub duration: Duration,
    /// Backend name (`"cli"` or `"native"`).
    pub backend: &'static str,
    /// Warnings printed while rendering.
    pub warnings: Vec<String>,
    /// Located warnings.
    pub diagnostics: Vec<Diagnostic>,
    /// SVG metadata, for SVG output.
    pub svg: Option<SvgInfo>,
    /// Hash of the source and options (see [`crate::fresh`]).
    pub source_hash: String,
    /// Raw stderr of the backend.
    pub stderr: String,
}

impl RenderReport {
    /// Output size in bytes, if the file can be read.
    pub fn output_len(&self) -> Option<u64> {
        std::fs::metadata(&self.output).ok().map(|m| m.len())
    }
}

impl fmt::Display for RenderReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rendered {} -> {} ({}, {} backend) in {:.1?}",
            self.input.display(),
            self.output.display(),
            self.format,
            self.backend,
            self.duration
        )?;
        if let Some(svg) = &self.svg {
            write!(f, "; {svg}")?;
        }
        for w in &self.warnings {
            write!(f, "\n  warning: {w}")?;
        }
        Ok(())
    }
}
