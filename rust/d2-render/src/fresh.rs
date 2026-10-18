//! Freshness: is a rendered file still up to date with its `.d2` source?
//!
//! A render with [`crate::RenderOptions::stamp`] records
//! `sha256(source bytes + "\0" + options fingerprint)`:
//! in an SVG as a trailing `<!-- d2-render:source-sha256=... -->` comment,
//! for other formats in a `<output>.d2hash` sidecar file. Only the main file
//! is hashed; changes in files it imports are not tracked.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{Format, RenderOptions, Result};

const SVG_MARK: &str = "d2-render:source-sha256=";

/// Hex SHA-256 of a source and the options fingerprint.
pub fn source_hash(source: &[u8], options: &RenderOptions) -> String {
    let mut h = Sha256::new();
    h.update(source);
    h.update([0u8]);
    h.update(options.fingerprint().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The sidecar used for non-SVG outputs: `out.png` -> `out.png.d2hash`.
pub fn sidecar_path(output: &Path) -> PathBuf {
    let mut s = output.as_os_str().to_owned();
    s.push(".d2hash");
    PathBuf::from(s)
}

/// The hash stamped in SVG text, if any.
pub fn stamp_in_svg(svg: &str) -> Option<String> {
    let i = svg.rfind(SVG_MARK)?;
    let rest = &svg[i + SVG_MARK.len()..];
    let hex: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    (hex.len() == 64).then_some(hex)
}

/// Record `hash` for `output` (SVG comment or sidecar).
pub fn write_stamp(output: &Path, format: Format, hash: &str) -> Result<()> {
    if format == Format::Svg {
        let mut svg = fs::read_to_string(output)?;
        if let Some(i) = svg.rfind("<!-- d2-render:") {
            if let Some(end) = svg[i..].find("-->") {
                svg.replace_range(i..i + end + 3, "");
            }
        }
        let trimmed = svg.trim_end().len();
        svg.truncate(trimmed);
        svg.push_str(&format!("\n<!-- {SVG_MARK}{hash} -->\n"));
        fs::write(output, svg)?;
    } else {
        fs::write(sidecar_path(output), format!("source-sha256={hash}\n"))?;
    }
    Ok(())
}

/// The hash recorded for `output`, looking at the SVG comment then the
/// sidecar. `Ok(None)` when the output exists but carries no stamp.
pub fn read_stamp(output: &Path) -> Result<Option<String>> {
    let side = sidecar_path(output);
    match fs::read_to_string(&side) {
        Ok(s) => {
            let hex = s.trim().trim_start_matches("source-sha256=").to_string();
            if hex.len() == 64 {
                return Ok(Some(hex));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    if Format::from_path(output).ok() == Some(Format::Svg) {
        let bytes = fs::read(output)?;
        return Ok(stamp_in_svg(&String::from_utf8_lossy(&bytes)));
    }
    Ok(None)
}

/// Why an output is (or is not) up to date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    /// The recorded hash matches the source.
    Fresh,
    /// The output file does not exist.
    Missing,
    /// The output exists but has no stamp, so freshness is unknown.
    Unstamped,
    /// The source (or the options) changed since the render.
    Outdated {
        /// Hash recorded in the output.
        recorded: String,
        /// Hash of the current source.
        current: String,
    },
}

impl Freshness {
    /// `true` unless [`Freshness::Fresh`].
    pub fn is_stale(&self) -> bool {
        !matches!(self, Freshness::Fresh)
    }
}

impl fmt::Display for Freshness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Freshness::Fresh => f.write_str("up to date"),
            Freshness::Missing => f.write_str("output missing"),
            Freshness::Unstamped => f.write_str("output has no source stamp"),
            Freshness::Outdated { .. } => f.write_str("source changed since last render"),
        }
    }
}

/// One output that needs re-rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleOutput {
    /// The `.d2` source.
    pub input: PathBuf,
    /// The rendered file.
    pub output: PathBuf,
    /// Why it is stale.
    pub freshness: Freshness,
}

impl fmt::Display for StaleOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} -> {}: {}",
            self.input.display(),
            self.output.display(),
            self.freshness
        )
    }
}

/// Compare `output` against `input` rendered with `options`.
pub fn check(input: &Path, output: &Path, options: &RenderOptions) -> Result<Freshness> {
    let source = fs::read(input)?;
    if !output.exists() {
        return Ok(Freshness::Missing);
    }
    let current = source_hash(&source, options);
    Ok(match read_stamp(output)? {
        None => Freshness::Unstamped,
        Some(recorded) if recorded == current => Freshness::Fresh,
        Some(recorded) => Freshness::Outdated { recorded, current },
    })
}

/// All `.d2` files under `dir` (recursive, skipping hidden directories and
/// `target`/`node_modules`), sorted.
pub fn find_sources(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                walk(&path, out)?;
            }
        } else if path.extension().is_some_and(|e| e == "d2") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_depends_on_options() {
        let a = source_hash(b"a -> b", &RenderOptions::default());
        let b = source_hash(b"a -> b", &RenderOptions::default().theme(1));
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn svg_stamp_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.svg");
        fs::write(&p, "<svg></svg>").unwrap();
        let h = "a".repeat(64);
        write_stamp(&p, Format::Svg, &h).unwrap();
        write_stamp(&p, Format::Svg, &h).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert_eq!(text.matches(SVG_MARK).count(), 1);
        assert_eq!(read_stamp(&p).unwrap(), Some(h));
    }
}
