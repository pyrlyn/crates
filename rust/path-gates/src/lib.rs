//! Map changed paths to named gates by glob rules.
//!
//! A config declares gates (`rust`, `docs`, ...) with glob patterns; [`Rules::select`] turns a
//! list of repo-relative changed paths into the set of gates that must run. When a path cannot
//! be attributed to any gate the answer is "every gate" ([`Selection::all`]), never "nothing".
//!
//! ```toml
//! [[gate]]
//! name = "rust"
//! paths = ["**/*.rs", "Cargo.lock"]
//!
//! [[gate]]
//! name = "docs"
//! paths = ["**/*.md"]
//!
//! unmatched = "all" # or "ignore"
//! ```
#![deny(missing_docs)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;

/// Why a config could not be turned into [`Rules`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The text is not valid TOML or does not match the config shape.
    #[error("invalid gate config: {0}")]
    Config(#[from] toml::de::Error),
    /// A glob pattern could not be compiled.
    #[error("gate `{gate}`: invalid glob `{pattern}`: {source}")]
    InvalidGlob {
        /// Gate that owns the pattern.
        gate: String,
        /// The offending pattern.
        pattern: String,
        /// Compiler error from `globset`.
        source: globset::Error,
    },
    /// Two gates share a name.
    #[error("duplicate gate name `{0}`")]
    DuplicateGate(String),
}

/// What to do with a changed path that no gate claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unmatched {
    /// Select every gate: an unknown path may affect anything.
    #[default]
    All,
    /// Drop the path; it is only reported in [`Selection::unmatched`].
    Ignore,
}

// Unknown keys are ignored on purpose: a host tool keeps its own keys (`run`, `base`, ...) in
// the same tables.
#[derive(Deserialize)]
struct RawConfig {
    #[serde(default)]
    gate: Vec<GateSpec>,
    #[serde(default)]
    unmatched: Unmatched,
}

/// One gate as declared in the config; a host can embed it in its own `Deserialize` struct and
/// pass the result to [`Rules::new`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GateSpec {
    /// Unique gate name.
    pub name: String,
    /// Glob patterns matched against `/`-separated repo-relative paths.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Select this gate whenever anything at all changed.
    #[serde(default)]
    pub always: bool,
}

struct Gate {
    name: String,
    globs: GlobSet,
    always: bool,
}

/// Compiled gate rules.
pub struct Rules {
    gates: Vec<Gate>,
    unmatched: Unmatched,
}

/// The gates a set of changed paths selects.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    /// Names of the gates to run.
    pub gates: BTreeSet<String>,
    /// Paths no gate claimed, in input order, normalised to `/` separators.
    pub unmatched: Vec<PathBuf>,
    /// True when an unmatched path forced every gate.
    pub all: bool,
}

impl Rules {
    /// Parse and compile a TOML config. Keys this crate does not know are ignored.
    ///
    /// # Errors
    /// [`Error::Config`] for malformed TOML or a wrongly typed known key, plus the errors of
    /// [`Rules::new`].
    pub fn from_toml(text: &str) -> Result<Self, Error> {
        let raw: RawConfig = toml::from_str(text)?;
        Self::new(raw.gate, raw.unmatched)
    }

    /// Compile gates built without TOML, e.g. from a host tool's own config struct.
    ///
    /// # Errors
    /// [`Error::InvalidGlob`] for a pattern that does not compile, [`Error::DuplicateGate`]
    /// for a repeated gate name.
    pub fn new(specs: Vec<GateSpec>, unmatched: Unmatched) -> Result<Self, Error> {
        let mut seen = BTreeSet::new();
        let mut gates = Vec::with_capacity(specs.len());
        for spec in specs {
            if !seen.insert(spec.name.clone()) {
                return Err(Error::DuplicateGate(spec.name));
            }
            gates.push(compile(spec)?);
        }
        Ok(Self { gates, unmatched })
    }

    /// Gate names in config order.
    pub fn gate_names(&self) -> impl Iterator<Item = &str> {
        self.gates.iter().map(|g| g.name.as_str())
    }

    /// Select the gates for repo-relative changed paths. Empty input selects nothing.
    ///
    /// Backslashes are read as separators so Windows-style input matches the same globs.
    pub fn select<I, P>(&self, paths: I) -> Selection
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut selection = Selection::default();
        let mut any = false;
        for path in paths {
            any = true;
            let normalised = normalise(path.as_ref());
            let mut claimed = false;
            for gate in self.gates.iter().filter(|g| g.globs.is_match(&normalised)) {
                claimed = true;
                selection.gates.insert(gate.name.clone());
            }
            if !claimed {
                selection.unmatched.push(PathBuf::from(&normalised));
                if self.unmatched == Unmatched::All {
                    selection.all = true;
                }
            }
        }
        if selection.all {
            selection.gates = self.gate_names().map(str::to_owned).collect();
        } else if any {
            let always = self.gates.iter().filter(|g| g.always);
            selection.gates.extend(always.map(|g| g.name.clone()));
        }
        selection
    }
}

fn compile(gate: GateSpec) -> Result<Gate, Error> {
    let mut builder = GlobSetBuilder::new();
    for pattern in &gate.paths {
        // `*` must not cross `/`, otherwise `*.md` would claim `a/b.md`.
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|source| Error::InvalidGlob {
                gate: gate.name.clone(),
                pattern: pattern.clone(),
                source,
            })?;
        builder.add(glob);
    }
    let globs = builder.build().map_err(|source| Error::InvalidGlob {
        gate: gate.name.clone(),
        pattern: gate.paths.join(", "),
        source,
    })?;
    Ok(Gate {
        name: gate.name,
        globs,
        always: gate.always,
    })
}

fn normalise(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => match parts.last() {
                None | Some(&"..") => parts.push(".."),
                Some(_) => {
                    parts.pop();
                }
            },
            other => parts.push(other),
        }
    }
    parts.join("/")
}
