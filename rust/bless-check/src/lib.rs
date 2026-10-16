//! A committed generated file (schema, header, bindings, table) must be what the
//! code renders now. [`check_or_bless`] compares the two and, when blessed,
//! rewrites the file instead of failing; [`assert_fresh`] is the test form.
//!
//! The comparison ignores CRLF versus LF: a Windows checkout may have turned the
//! committed file's line endings into CRLF, and the text is still the same.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Longest line excerpt a [`Error::Differs`] carries, in characters.
const EXCERPT_CHARS: usize = 160;

/// What to do when the committed file is not what was rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Fail: a missing or different file is an error.
    Check,
    /// Write a missing file (its first run), fail on a different one.
    CreateMissing,
    /// Write a missing or different file.
    Bless,
}

impl Mode {
    /// [`Mode::Bless`] when the environment variable `var` is set to anything, else
    /// [`Mode::Check`]: `FOO_BLESS=1 cargo test` rewrites, plain `cargo test` checks.
    pub fn from_env(var: &str) -> Mode {
        if std::env::var_os(var).is_some() {
            Mode::Bless
        } else {
            Mode::Check
        }
    }
}

/// A run that did not fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The committed file already matched; nothing was written.
    Fresh,
    /// The file was missing or different and was written.
    Written,
}

/// Why the committed file is not fresh, or why it could not be read or written.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// No committed file at `path`.
    Missing { path: PathBuf },
    /// The texts first differ at 1-based `line`; `None` is a side that ended before it.
    Differs {
        path: PathBuf,
        line: usize,
        committed: Option<String>,
        rendered: Option<String>,
    },
    /// Reading or writing `path` failed for another reason than its absence.
    Io {
        path: PathBuf,
        action: &'static str,
        source: io::Error,
    },
}

impl Error {
    /// The file is missing or different, as opposed to an I/O failure.
    pub fn is_stale(&self) -> bool {
        matches!(self, Error::Missing { .. } | Error::Differs { .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Missing { path } => write!(f, "{} is missing", path.display()),
            Error::Differs {
                path,
                line,
                committed,
                rendered,
            } => {
                let side = |s: &Option<String>| {
                    s.as_deref()
                        .map_or("<end of file>".into(), |l| format!("{l:?}"))
                };
                write!(
                    f,
                    "{} is stale: line {line} differs\n  committed: {}\n  rendered:  {}",
                    path.display(),
                    side(committed),
                    side(rendered)
                )
            }
            Error::Io {
                path,
                action,
                source,
            } => write!(f, "{action} {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Compares the committed file at `path` with `rendered`, and writes `rendered`
/// there (creating parent directories) when `mode` allows it. An unchanged file
/// is never rewritten, so its modification time only moves when its text does.
pub fn check_or_bless(
    path: impl AsRef<Path>,
    rendered: impl AsRef<[u8]>,
    mode: Mode,
) -> Result<Outcome, Error> {
    let path = path.as_ref();
    let rendered = rendered.as_ref();
    let stale = match fs::read(path) {
        Ok(committed) => match first_difference(&committed, rendered) {
            None => return Ok(Outcome::Fresh),
            Some((line, committed, rendered)) => Error::Differs {
                path: path.to_owned(),
                line,
                committed,
                rendered,
            },
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Error::Missing {
            path: path.to_owned(),
        },
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                action: "reading",
                source,
            });
        }
    };
    let write = match mode {
        Mode::Bless => true,
        Mode::CreateMissing => matches!(stale, Error::Missing { .. }),
        Mode::Check => false,
    };
    if !write {
        return Err(stale);
    }
    write_creating_dirs(path, rendered)?;
    Ok(Outcome::Written)
}

/// Test form of [`check_or_bless`]: panics with the difference and `hint`, the
/// command that regenerates the file.
#[track_caller]
pub fn assert_fresh(path: impl AsRef<Path>, rendered: impl AsRef<[u8]>, mode: Mode, hint: &str) {
    if let Err(e) = check_or_bless(path, rendered, mode) {
        panic!("{e}\n{hint}");
    }
}

fn write_creating_dirs(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let io_error = |action, source| Error::Io {
        path: path.to_owned(),
        action,
        source,
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(|e| io_error("creating the directory of", e))?;
    }
    fs::write(path, bytes).map_err(|e| io_error("writing", e))
}

/// The first line on which the texts differ once CRLF is read as LF, with an
/// excerpt of each side; `None` when they are the same text.
fn first_difference(
    committed: &[u8],
    rendered: &[u8],
) -> Option<(usize, Option<String>, Option<String>)> {
    let committed = lf_lines(committed);
    let rendered = lf_lines(rendered);
    if committed == rendered {
        return None;
    }
    let index = committed
        .iter()
        .zip(&rendered)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| committed.len().min(rendered.len()));
    let excerpt = |lines: &[&[u8]]| lines.get(index).map(|l| excerpt(l));
    Some((index + 1, excerpt(&committed), excerpt(&rendered)))
}

/// Lines split on LF, the CR of each CRLF dropped; a final newline yields a final
/// empty line, so a missing last newline still counts as a difference.
fn lf_lines(text: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = text.split(|b| *b == b'\n').collect();
    let terminated = lines.len() - 1;
    for line in &mut lines[..terminated] {
        *line = line.strip_suffix(b"\r").unwrap_or(line);
    }
    lines
}

fn excerpt(line: &[u8]) -> String {
    let text = String::from_utf8_lossy(line);
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(EXCERPT_CHARS).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}
