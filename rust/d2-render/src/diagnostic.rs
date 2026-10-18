//! Parsing of d2's human-readable stderr into structured diagnostics.
//!
//! The d2 CLI has no machine-readable output mode. Every line it prints on
//! stderr starts with a level prefix (`err:`, `warn:`, `info:`, `success:`)
//! and compile errors carry `path:line:col: message`. This module turns that
//! text into [`Diagnostic`]s and keeps everything else as plain messages.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// Compilation failed.
    Error,
    /// Rendering succeeded but something was off.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        })
    }
}

/// One problem reported for a diagram source, with its position when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Error or warning.
    pub severity: Severity,
    /// The file d2 named, if any (d2 omits it for `d2 validate`).
    pub path: Option<PathBuf>,
    /// 1-based line.
    pub line: Option<u32>,
    /// 1-based column.
    pub column: Option<u32>,
    /// The message without prefix or position.
    pub message: String,
}

impl Diagnostic {
    /// An error at `line:column`.
    pub fn error(line: u32, column: u32, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            path: None,
            line: Some(line),
            column: Some(column),
            message: message.into(),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(p) = &self.path {
            write!(f, "{}:", p.display())?;
        }
        if let (Some(l), Some(c)) = (self.line, self.column) {
            write!(f, "{l}:{c}: ")?;
        } else if self.path.is_some() {
            f.write_str(" ")?;
        }
        f.write_str(&self.message)
    }
}

/// Everything recognised in one d2 stderr stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedOutput {
    /// Located errors (`file:line:col: message`).
    pub diagnostics: Vec<Diagnostic>,
    /// `err:` lines without a position.
    pub errors: Vec<String>,
    /// `warn:` lines.
    pub warnings: Vec<String>,
    /// `info:` lines.
    pub infos: Vec<String>,
    /// The `success:` line, if any.
    pub success: Option<String>,
    /// Compile time d2 reported in its success line.
    pub reported_duration: Option<Duration>,
    /// d2 wrote a partial render despite failing.
    pub partial: bool,
}

/// Parse a d2 stderr stream.
pub fn parse_stderr(stderr: &str) -> ParsedOutput {
    let mut out = ParsedOutput::default();
    for raw in stderr.lines() {
        let line = raw.trim_end();
        if let Some(rest) = line.strip_prefix("err:") {
            let msg = rest.trim();
            if msg.is_empty() {
                continue;
            }
            if msg.contains("partial render written") {
                out.partial = true;
            }
            match parse_located(msg, Severity::Error) {
                Some(d) => out.diagnostics.push(d),
                None => out.errors.push(msg.to_string()),
            }
        } else if let Some(rest) = line.strip_prefix("warn:") {
            let msg = rest.trim();
            if let Some(d) = parse_located(msg, Severity::Warning) {
                out.diagnostics.push(d);
            }
            out.warnings.push(msg.to_string());
        } else if let Some(rest) = line.strip_prefix("info:") {
            out.infos.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("success:") {
            let msg = rest.trim().to_string();
            out.reported_duration = parse_trailing_duration(&msg);
            out.success = Some(msg);
        }
    }
    out
}

/// Find `[path:]line:col: message` in an error line.
///
/// d2 prefixes the location with context such as `failed to compile x.d2: `
/// or `github.com/d2lang/d2/d2cli.validateCmd: `; the location is the first
/// `digits:digits: ` that starts the line or follows `:` or a space.
pub fn parse_located(msg: &str, severity: Severity) -> Option<Diagnostic> {
    let bytes = msg.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let at_boundary = i == 0 || bytes[i - 1] == b':' || bytes[i - 1] == b' ';
        if at_boundary && bytes[i].is_ascii_digit() {
            if let Some((line, col, end)) = scan_line_col(bytes, i) {
                let before = &msg[..i];
                let path = before.strip_suffix(':').and_then(|p| {
                    let p = p.rsplit(": ").next().unwrap_or(p).trim();
                    (!p.is_empty()).then(|| PathBuf::from(p))
                });
                return Some(Diagnostic {
                    severity,
                    path,
                    line: Some(line),
                    column: Some(col),
                    message: msg[end..].trim().to_string(),
                });
            }
        }
        i += 1;
    }
    None
}

fn scan_digits(bytes: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == start {
        return None;
    }
    let n = std::str::from_utf8(&bytes[start..end]).ok()?.parse().ok()?;
    Some((n, end))
}

fn scan_line_col(bytes: &[u8], start: usize) -> Option<(u32, u32, usize)> {
    let (line, p) = scan_digits(bytes, start)?;
    if bytes.get(p) != Some(&b':') {
        return None;
    }
    let (col, q) = scan_digits(bytes, p + 1)?;
    if bytes.get(q) != Some(&b':') || bytes.get(q + 1) != Some(&b' ') {
        return None;
    }
    Some((line, col, q + 2))
}

/// `... in 4.394709ms` -> 4.394709 ms.
fn parse_trailing_duration(msg: &str) -> Option<Duration> {
    let last = msg.rsplit(' ').next()?;
    let (num, unit_secs) = if let Some(n) = last.strip_suffix("ms") {
        (n, 1e-3)
    } else if let Some(n) = last.strip_suffix("µs") {
        (n, 1e-6)
    } else if let Some(n) = last.strip_suffix("us") {
        (n, 1e-6)
    } else if let Some(n) = last.strip_suffix("ns") {
        (n, 1e-9)
    } else {
        (last.strip_suffix('s')?, 1.0)
    };
    let v: f64 = num.parse().ok()?;
    (v.is_finite() && v >= 0.0).then(|| Duration::from_secs_f64(v * unit_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_error_with_path() {
        let p = parse_stderr(
            "err: failed to compile bad.d2: /tmp/d2p/bad.d2:5:4: maps must be terminated with }\n",
        );
        assert_eq!(p.diagnostics.len(), 1);
        let d = &p.diagnostics[0];
        assert_eq!(
            d.path.as_deref(),
            Some(std::path::Path::new("/tmp/d2p/bad.d2"))
        );
        assert_eq!((d.line, d.column), (Some(5), Some(4)));
        assert_eq!(d.message, "maps must be terminated with }");
    }

    #[test]
    fn multiple_errors_and_continuation_lines() {
        let p = parse_stderr(concat!(
            "err: failed to compile sem3.d2: /x/sem3.d2:1:12: unknown shape \"nosuch\"\n",
            "err: /x/sem3.d2:2:18: expected \"opacity\" to be a number between 0.0 and 1.0\n",
        ));
        assert_eq!(p.diagnostics.len(), 2);
        assert_eq!(p.diagnostics[1].line, Some(2));
        assert_eq!(p.diagnostics[1].column, Some(18));
        assert_eq!(
            p.diagnostics[1].path.as_deref(),
            Some(std::path::Path::new("/x/sem3.d2"))
        );
    }

    #[test]
    fn validate_error_without_path() {
        let p = parse_stderr(
            "err: github.com/d2lang/d2/d2cli.validateCmd: 3:1: connection missing destination\n",
        );
        let d = &p.diagnostics[0];
        assert_eq!(d.path, None);
        assert_eq!((d.line, d.column), (Some(3), Some(1)));
        assert_eq!(d.message, "connection missing destination");
    }

    #[test]
    fn windows_path() {
        let d = parse_located(
            r"failed to compile a.d2: C:\x\a.d2:2:3: boom",
            Severity::Error,
        )
        .unwrap();
        assert_eq!(d.path.as_deref(), Some(std::path::Path::new(r"C:\x\a.d2")));
        assert_eq!(d.message, "boom");
    }

    #[test]
    fn success_and_partial() {
        let p = parse_stderr(concat!(
            "success: successfully compiled ok.d2 to ok.svg in 4.5ms\n",
            "warn: something odd\n",
            "err: failed to fully compile (partial render written) w2.d2: failed to bundle\n",
        ));
        assert!(p.partial);
        assert_eq!(p.warnings, vec!["something odd".to_string()]);
        assert_eq!(p.reported_duration, Some(Duration::from_micros(4500)));
        assert_eq!(p.errors.len(), 1);
    }

    #[test]
    fn display() {
        let d = Diagnostic::error(3, 1, "x");
        assert_eq!(d.to_string(), "3:1: x");
    }
}
