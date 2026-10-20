//! The crate's error type.

use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::diagnostic::Diagnostic;
use crate::Format;

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong while rendering or validating a diagram.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The `d2` executable could not be found or started.
    BinaryNotFound {
        /// The binary that was looked up (explicit path, `$D2_BIN`, or `d2`).
        binary: PathBuf,
        /// Why it was rejected.
        reason: String,
    },
    /// The diagram source does not compile; `diagnostics` holds every error
    /// D2 reported, with line and column when it gave them.
    Syntax {
        /// Parsed diagnostics, in the order D2 printed them.
        diagnostics: Vec<Diagnostic>,
        /// The raw standard error, untouched.
        stderr: String,
    },
    /// Reading or writing a file failed.
    Io(io::Error),
    /// `d2` exited with a non-zero status for a reason other than a
    /// compile error (bad flag, unknown theme or layout, timeout, ...).
    Failed {
        /// The exit code, if the process was not killed by a signal.
        code: Option<i32>,
        /// Error lines from stderr with the `err:` prefix removed.
        messages: Vec<String>,
        /// The raw standard error.
        stderr: String,
        /// `true` when d2 still wrote a partial render (for example when a
        /// remote icon could not be fetched).
        partial: bool,
    },
    /// The backend cannot produce this format.
    UnsupportedFormat {
        /// The requested format.
        format: Format,
        /// The backend name.
        backend: &'static str,
    },
    /// The output path has an extension that maps to no known format.
    UnknownFormat(PathBuf),
    /// The file watcher failed (only with the `watch` feature).
    Watch(String),
    /// Rasterizing SVG to PNG failed (native backend, feature `png`).
    Raster(String),
}

impl Error {
    /// The diagnostics carried by a [`Error::Syntax`], empty otherwise.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            Error::Syntax { diagnostics, .. } => diagnostics,
            _ => &[],
        }
    }

    /// `true` for [`Error::BinaryNotFound`].
    pub fn is_binary_not_found(&self) -> bool {
        matches!(self, Error::BinaryNotFound { .. })
    }

    /// `true` for [`Error::Syntax`].
    pub fn is_syntax(&self) -> bool {
        matches!(self, Error::Syntax { .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BinaryNotFound { binary, reason } => write!(
                f,
                "d2 binary not found ({}): {reason}; install d2 (https://d2lang.com) or set D2_BIN",
                binary.display()
            ),
            Error::Syntax { diagnostics, .. } => {
                write!(f, "d2 compile failed")?;
                for (i, d) in diagnostics.iter().enumerate() {
                    let sep = if i == 0 { ": " } else { "; " };
                    write!(f, "{sep}{d}")?;
                }
                Ok(())
            }
            Error::Io(e) => write!(f, "i/o error: {e}"),
            Error::Failed {
                code,
                messages,
                partial,
                ..
            } => {
                match code {
                    Some(c) => write!(f, "d2 exited with status {c}")?,
                    None => write!(f, "d2 was terminated by a signal")?,
                }
                if *partial {
                    write!(f, " (partial render written)")?;
                }
                if let Some(first) = messages.first() {
                    write!(f, ": {first}")?;
                }
                Ok(())
            }
            Error::UnsupportedFormat { format, backend } => {
                write!(f, "the {backend} backend cannot render {format}")
            }
            Error::UnknownFormat(p) => {
                write!(f, "cannot infer an output format from {}", p.display())
            }
            Error::Watch(msg) => write!(f, "file watcher error: {msg}"),
            Error::Raster(msg) => write!(f, "png rasterization failed: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
