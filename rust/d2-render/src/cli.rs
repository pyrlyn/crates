//! Backend that runs the `d2` executable.

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::backend::{Backend, BackendOutput};
use crate::diagnostic::parse_stderr;
use crate::{Error, Format, RenderOptions, Result};

/// Environment variable that overrides the d2 executable.
pub const D2_BIN_ENV: &str = "D2_BIN";

/// Renders through the `d2` CLI.
///
/// The binary is, in order: the path given to [`CliBackend::with_binary`],
/// `$D2_BIN`, or `d2` looked up on `$PATH`. Resolution happens on every call,
/// so a missing binary is reported as [`Error::BinaryNotFound`] by the call
/// that needs it, not when the backend is built.
#[derive(Debug, Clone, Default)]
pub struct CliBackend {
    binary: Option<PathBuf>,
}

impl CliBackend {
    /// Resolve the binary from `$D2_BIN` or `$PATH` at call time.
    pub fn new() -> Self {
        Self::default()
    }

    /// Always use this binary (a path, or a bare name looked up on `$PATH`).
    pub fn with_binary(path: impl Into<PathBuf>) -> Self {
        Self {
            binary: Some(path.into()),
        }
    }

    /// The executable that would run now.
    pub fn resolve_binary(&self) -> Result<PathBuf> {
        let wanted = match &self.binary {
            Some(p) => p.clone(),
            None => match env::var_os(D2_BIN_ENV).filter(|v| !v.is_empty()) {
                Some(v) => PathBuf::from(v),
                None => PathBuf::from("d2"),
            },
        };
        find_executable(&wanted).ok_or_else(|| Error::BinaryNotFound {
            reason: if wanted.components().count() > 1 {
                "no such file".to_string()
            } else {
                "not found on PATH".to_string()
            },
            binary: wanted,
        })
    }

    fn run(&self, args: &[OsString]) -> Result<Output> {
        let bin = self.resolve_binary()?;
        Command::new(&bin)
            .args(args)
            // d2 colours its output when it thinks it is on a terminal.
            .env("NO_COLOR", "1")
            .output()
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
                    Error::BinaryNotFound {
                        binary: bin.clone(),
                        reason: e.to_string(),
                    }
                }
                _ => Error::Io(e),
            })
    }

    /// Only `d2 validate`: parse errors, no semantic checks (see
    /// [`Backend::validate`] for the full check).
    pub fn validate_syntax(&self, input: &Path) -> Result<()> {
        let input = std::path::absolute(input)?;
        let out = self.run(&["validate".into(), input.into_os_string()])?;
        check_status(&out).map(|_| ())
    }

    /// Run `d2 fmt` on `input`, rewriting it in place.
    pub fn format_file(&self, input: &Path) -> Result<()> {
        let out = self.run(&["fmt".into(), input.as_os_str().to_owned()])?;
        check_status(&out).map(|_| ())
    }

    /// `true` when `input` is already formatted (`d2 fmt --check`).
    pub fn is_formatted(&self, input: &Path) -> Result<bool> {
        let out = self.run(&["fmt".into(), "--check".into(), input.as_os_str().to_owned()])?;
        if out.status.success() {
            return Ok(true);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let parsed = parse_stderr(&stderr);
        if parsed.diagnostics.is_empty() && stderr.contains("unformatted file") {
            return Ok(false);
        }
        check_status(&out).map(|_| false)
    }
}

impl Backend for CliBackend {
    fn name(&self) -> &'static str {
        "cli"
    }

    fn supports(&self, _format: Format) -> bool {
        true
    }

    fn render(
        &self,
        input: &Path,
        output: &Path,
        _format: Format,
        options: &RenderOptions,
    ) -> Result<BackendOutput> {
        let mut args = options.to_args();
        // Watch mode would never return; d2 reads it from the environment.
        args.push("--watch=false".into());
        // Absolute paths, so a sandboxed d2 (e.g. a WASI build under
        // wasmtime, whose working directory is `/`) finds the files.
        args.push(std::path::absolute(input)?.into_os_string());
        args.push(std::path::absolute(output)?.into_os_string());
        let out = self.run(&args)?;
        let stderr = check_status(&out)?;
        let parsed = parse_stderr(&stderr);
        Ok(BackendOutput {
            warnings: parsed.warnings,
            diagnostics: parsed.diagnostics,
            version: None,
            stderr,
        })
    }

    /// `d2 validate`, then a full compile to stdout.
    ///
    /// `d2 validate` (v0.9) only parses: an unknown shape or an out-of-range
    /// style value passes it. The second step compiles and lays out the
    /// diagram without writing a file (and without fetching remote icons,
    /// `--bundle=false`), so every error the renderer would hit is reported.
    fn validate(&self, input: &Path) -> Result<()> {
        let input = std::path::absolute(input)?;
        self.validate_syntax(&input)?;
        let out = self.run(&[
            "--watch=false".into(),
            "--bundle=false".into(),
            "--stdout-format=svg".into(),
            input.into_os_string(),
            "-".into(),
        ])?;
        check_status(&out).map(|_| ())
    }

    fn version(&self) -> Result<String> {
        let out = self.run(&["--version".into()])?;
        check_status(&out)?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

/// Map a finished process to `Ok(stderr)` or the matching error.
fn check_status(out: &Output) -> Result<String> {
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        return Ok(stderr);
    }
    let parsed = parse_stderr(&stderr);
    let errors: Vec<_> = parsed
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::Severity::Error)
        .cloned()
        .collect();
    if !errors.is_empty() && !parsed.partial {
        return Err(Error::Syntax {
            diagnostics: errors,
            stderr,
        });
    }
    let mut messages = parsed.errors;
    if messages.is_empty() && !stderr.trim().is_empty() {
        messages.push(stderr.trim().to_string());
    }
    Err(Error::Failed {
        code: out.status.code(),
        messages,
        stderr,
        partial: parsed.partial,
    })
}

/// Resolve `name` like a shell would: a path with a directory part must
/// exist; a bare name is searched in `$PATH` (with `PATHEXT` on Windows).
pub(crate) fn find_executable(name: &Path) -> Option<PathBuf> {
    if name.components().count() > 1 || name.is_absolute() {
        return is_executable(name).then(|| name.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        for candidate in candidates(&dir.join(name)) {
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn candidates(base: &Path) -> Vec<PathBuf> {
    let mut v = vec![base.to_path_buf()];
    let exts = env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
    for ext in exts.split(';').filter(|e| !e.is_empty()) {
        let mut s = base.as_os_str().to_owned();
        s.push(ext);
        v.push(PathBuf::from(s));
    }
    v
}

#[cfg(not(windows))]
fn candidates(base: &Path) -> Vec<PathBuf> {
    vec![base.to_path_buf()]
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}
