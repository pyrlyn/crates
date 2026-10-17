//! The high-level entry point.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::backend::Backend;
#[cfg(feature = "cli")]
use crate::cli::CliBackend;
use crate::fresh::{self, Freshness, StaleOutput};
use crate::report::RenderReport;
use crate::svg::SvgInfo;
use crate::{Error, Format, RenderOptions, Result};

/// Renders D2 sources through a [`Backend`]: the pure-Rust native backend
/// by default, or the `d2` CLI with feature `cli`.
///
/// Cheap to clone: the backend is shared.
#[derive(Debug, Clone)]
pub struct Renderer {
    backend: Arc<dyn Backend>,
    options: RenderOptions,
}

#[cfg(any(feature = "native", feature = "cli"))]
impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// The default backend: [`crate::NativeBackend`] when feature `native`
    /// is on (the default), otherwise the `d2` CLI (feature `cli`).
    #[cfg(any(feature = "native", feature = "cli"))]
    pub fn new() -> Self {
        #[cfg(feature = "native")]
        {
            Self::native()
        }
        #[cfg(not(feature = "native"))]
        {
            Self::cli()
        }
    }

    /// CLI backend; the binary is `$D2_BIN` or `d2` on `$PATH` (feature `cli`).
    #[cfg(feature = "cli")]
    pub fn cli() -> Self {
        Self::with_backend(CliBackend::new())
    }

    /// CLI backend with an explicit binary path (feature `cli`).
    #[cfg(feature = "cli")]
    pub fn with_binary(path: impl Into<PathBuf>) -> Self {
        Self::with_backend(CliBackend::with_binary(path))
    }

    /// Any backend.
    pub fn with_backend(backend: impl Backend + 'static) -> Self {
        Self {
            backend: Arc::new(backend),
            options: RenderOptions::default(),
        }
    }

    /// Pure-Rust backend (feature `native`, on by default).
    #[cfg(feature = "native")]
    pub fn native() -> Self {
        Self::with_backend(crate::native::NativeBackend::new())
    }

    /// Replace the render options.
    pub fn options(mut self, options: RenderOptions) -> Self {
        self.options = options;
        self
    }

    /// The current options.
    pub fn render_options(&self) -> &RenderOptions {
        &self.options
    }

    /// The backend in use.
    pub fn backend(&self) -> &dyn Backend {
        &*self.backend
    }

    /// Render `input` to `output` as `format`, returning the written path.
    ///
    /// d2 chooses the format from the extension; when `output`'s extension
    /// does not match `format`, the render goes to a temporary file that is
    /// then moved to `output`, so the file always holds `format`.
    pub fn render_file(&self, input: &Path, output: &Path, format: Format) -> Result<PathBuf> {
        self.render_file_report(input, output, format)
            .map(|r| r.output)
    }

    /// Like [`Renderer::render_file`], with the format taken from `output`'s
    /// extension.
    pub fn render_auto(&self, input: &Path, output: &Path) -> Result<RenderReport> {
        self.render_file_report(input, output, Format::from_path(output)?)
    }

    /// Render a source string; it is written to a temporary `.d2` file.
    pub fn render_str(&self, source: &str, output: &Path, format: Format) -> Result<PathBuf> {
        self.render_str_report(source, output, format)
            .map(|r| r.output)
    }

    /// [`Renderer::render_str`] with a full report.
    pub fn render_str_report(
        &self,
        source: &str,
        output: &Path,
        format: Format,
    ) -> Result<RenderReport> {
        let tmp = tempfile::Builder::new()
            .prefix("d2-render-")
            .suffix(".d2")
            .tempfile()?;
        fs::write(tmp.path(), source)?;
        self.render_file_report(tmp.path(), output, format)
    }

    /// [`Renderer::render_file`] with a full report.
    pub fn render_file_report(
        &self,
        input: &Path,
        output: &Path,
        format: Format,
    ) -> Result<RenderReport> {
        let started = Instant::now();
        if !self.backend.supports(format) {
            return Err(Error::UnsupportedFormat {
                format,
                backend: self.backend.name(),
            });
        }
        let source = fs::read(input)?;
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let ext_matches = output
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(format.extension()));
        let out = if ext_matches {
            self.backend.render(input, output, format, &self.options)?
        } else {
            let dir = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let tmp = tempfile::Builder::new()
                .prefix(".d2-render-")
                .suffix(&format!(".{}", format.extension()))
                .tempfile_in(dir)?;
            let out = self
                .backend
                .render(input, tmp.path(), format, &self.options)?;
            tmp.persist(output).map_err(|e| Error::Io(e.error))?;
            out
        };
        let hash = fresh::source_hash(&source, &self.options);
        if self.options.stamp {
            fresh::write_stamp(output, format, &hash)?;
        }
        let svg = if format == Format::Svg {
            Some(SvgInfo::from_file(output)?)
        } else {
            None
        };
        Ok(RenderReport {
            input: input.to_path_buf(),
            output: output.to_path_buf(),
            format,
            duration: started.elapsed(),
            backend: self.backend.name(),
            warnings: out.warnings,
            diagnostics: out.diagnostics,
            svg,
            source_hash: hash,
            stderr: out.stderr,
        })
    }

    /// Check that `input` compiles (`d2 validate` for the CLI backend).
    /// A compile error is [`Error::Syntax`] with diagnostics.
    pub fn validate_file(&self, input: &Path) -> Result<()> {
        self.backend.validate(input)
    }

    /// [`Renderer::validate_file`] for a source string.
    pub fn validate_str(&self, source: &str) -> Result<()> {
        let tmp = tempfile::Builder::new()
            .prefix("d2-render-")
            .suffix(".d2")
            .tempfile()?;
        fs::write(tmp.path(), source)?;
        self.backend.validate(tmp.path())
    }

    /// Version of the backend's renderer (e.g. `v0.9.0`).
    pub fn version(&self) -> Result<String> {
        self.backend.version()
    }

    /// Freshness of `output` against `input` and these options.
    pub fn freshness(&self, input: &Path, output: &Path) -> Result<Freshness> {
        fresh::check(input, output, &self.options)
    }

    /// `true` when `output` is missing, unstamped, or older than `input`.
    pub fn is_stale(&self, input: &Path, output: &Path) -> Result<bool> {
        Ok(self.freshness(input, output)?.is_stale())
    }

    /// The outputs among `pairs` (input, output) that are stale.
    pub fn verify<I, P, Q>(&self, pairs: I) -> Result<Vec<StaleOutput>>
    where
        I: IntoIterator<Item = (P, Q)>,
        P: AsRef<Path>,
        Q: AsRef<Path>,
    {
        let mut stale = Vec::new();
        for (i, o) in pairs {
            let freshness = self.freshness(i.as_ref(), o.as_ref())?;
            if freshness.is_stale() {
                stale.push(StaleOutput {
                    input: i.as_ref().to_path_buf(),
                    output: o.as_ref().to_path_buf(),
                    freshness,
                });
            }
        }
        Ok(stale)
    }

    /// Every `.d2` under `dir` whose sibling `<stem>.<format>` is stale.
    pub fn verify_dir(&self, dir: &Path, format: Format) -> Result<Vec<StaleOutput>> {
        let sources = fresh::find_sources(dir)?;
        self.verify(sources.into_iter().map(|s| {
            let o = s.with_extension(format.extension());
            (s, o)
        }))
    }

    /// Re-render every stale output under `dir`; returns the reports.
    /// Stamping is forced on so the next check finds them fresh.
    pub fn refresh_dir(&self, dir: &Path, format: Format) -> Result<Vec<RenderReport>> {
        let stamped = self.clone().options(RenderOptions {
            stamp: true,
            ..self.options.clone()
        });
        let mut reports = Vec::new();
        for s in stamped.verify_dir(dir, format)? {
            reports.push(stamped.render_file_report(&s.input, &s.output, format)?);
        }
        Ok(reports)
    }
}
