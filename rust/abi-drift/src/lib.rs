//! Drift tests for generated bindings: regenerate a C header, C# bindings or
//! any other generated file in a test, compare it with the committed copy,
//! and show a unified diff when they differ. With the bless switch on, the
//! test rewrites the committed copy instead.
//!
//! A test rather than a script, so every platform's test run catches a stale
//! header with no tool to install. Line endings are compared as LF, because
//! a Windows checkout may have turned the committed file, or an input such as
//! `cbindgen.toml`'s header, into CRLF. Extracted from scull's `scull-ffi`
//! and ketch's `ketch-capi` drift tests.
//!
//! ```no_run
//! # fn main() -> Result<(), abi_drift::Error> {
//! let dir = env!("CARGO_MANIFEST_DIR");
//! let header = abi_drift::cbindgen_header(dir, "cbindgen.toml", "src/lib.rs")?;
//! abi_drift::Drift::new(dir)
//!     .bless_from_env("MYAPP_BLESS")
//!     .regenerate_with("MYAPP_BLESS=1 cargo test")
//!     .check("include/myapp.h", &header)?;
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Why a check or a regeneration failed.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The committed file differs from what the generator renders.
    #[error("{path} is stale: run `{hint}`\n{diff}")]
    Stale {
        /// The committed file, relative to the checker's directory.
        path: String,
        /// The command that regenerates it.
        hint: String,
        /// Unified diff from the committed text to the generated text.
        diff: String,
    },
    /// Reading or blessing the committed file failed.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: String,
        /// The underlying error.
        source: io::Error,
    },
    /// A generator rejected its input.
    #[error("generating bindings: {0}")]
    Generate(String),
}

// A failing test prints its error with `Debug`; the diff is only readable in
// the `Display` form, so `Debug` is the same text.
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// What a passing [`Drift::check`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The committed file matches.
    Fresh,
    /// The bless switch was on and the committed file was rewritten.
    Blessed,
}

/// Compares generated text with committed files under one directory.
#[derive(Debug, Clone)]
pub struct Drift {
    dir: PathBuf,
    bless: bool,
    hint: String,
}

impl Drift {
    /// Committed files are relative to `dir`, usually `CARGO_MANIFEST_DIR`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            bless: false,
            hint: "the drift test with its bless variable set".to_owned(),
        }
    }

    /// Rewrite instead of compare.
    #[must_use]
    pub fn bless(mut self, on: bool) -> Self {
        self.bless = on;
        self
    }

    /// Rewrite instead of compare when the environment variable `var` is
    /// set, and name it in the stale message.
    #[must_use]
    pub fn bless_from_env(self, var: &str) -> Self {
        let on = std::env::var_os(var).is_some();
        let hint = format!("{var}=1 cargo test");
        self.bless(on).regenerate_with(hint)
    }

    /// The command the stale message tells the reader to run.
    #[must_use]
    pub fn regenerate_with(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }

    /// Compares `rendered` with the committed `relative`, or rewrites it when
    /// blessing. A missing committed file reads as empty, so the diff shows
    /// the whole file to add.
    ///
    /// # Errors
    ///
    /// [`Error::Stale`] with a unified diff when the texts differ, or
    /// [`Error::Io`] when the file cannot be read or written.
    pub fn check(&self, relative: impl AsRef<Path>, rendered: &str) -> Result<Outcome, Error> {
        let relative = relative.as_ref();
        let path = self.dir.join(relative);
        let label = relative.display().to_string();
        let io = |source| Error::Io {
            path: label.clone(),
            source,
        };
        if self.bless {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
            std::fs::write(&path, rendered).map_err(io)?;
            return Ok(Outcome::Blessed);
        }
        let committed = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(io(e)),
        };
        let (committed, rendered) = (lf(&committed), lf(rendered));
        if committed == rendered {
            return Ok(Outcome::Fresh);
        }
        let diff = similar::TextDiff::from_lines(&committed, &rendered)
            .unified_diff()
            .context_radius(3)
            .header(
                &format!("{label} (committed)"),
                &format!("{label} (generated)"),
            )
            .to_string();
        Err(Error::Stale {
            path: label,
            hint: self.hint.clone(),
            diff,
        })
    }
}

fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The C header cbindgen generates for the crate in `crate_dir`, with the
/// config file and crate root given relative to it. cbindgen follows `mod`
/// items from the root by itself.
///
/// # Errors
///
/// [`Error::Generate`] when the config does not load or cbindgen fails.
#[cfg(feature = "cbindgen")]
pub fn cbindgen_header(
    crate_dir: impl AsRef<Path>,
    config: impl AsRef<Path>,
    src: impl AsRef<Path>,
) -> Result<String, Error> {
    let crate_dir = crate_dir.as_ref();
    let config = cbindgen::Config::from_file(crate_dir.join(config)).map_err(Error::Generate)?;
    let mut out = Vec::new();
    cbindgen::Builder::new()
        .with_config(config)
        .with_src(crate_dir.join(src))
        .generate()
        .map_err(|e| Error::Generate(e.to_string()))?
        .write(&mut out);
    String::from_utf8(out).map_err(|e| Error::Generate(e.to_string()))
}

/// The C# file a configured csbindgen `builder` generates, written to
/// `scratch` (a test's `CARGO_TARGET_TMPDIR`, say) and read back. csbindgen
/// reads each input file alone, so the builder lists every module with
/// exports.
///
/// # Errors
///
/// [`Error::Generate`] when csbindgen fails, [`Error::Io`] when the output
/// cannot be read back.
#[cfg(feature = "csbindgen")]
pub fn csbindgen_file(
    builder: csbindgen::Builder,
    scratch: impl AsRef<Path>,
) -> Result<String, Error> {
    let scratch = scratch.as_ref();
    builder
        .generate_csharp_file(scratch)
        .map_err(|e| Error::Generate(e.to_string()))?;
    std::fs::read_to_string(scratch).map_err(|source| Error::Io {
        path: scratch.display().to_string(),
        source,
    })
}
