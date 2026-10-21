//! `scoped-check.toml`: this tool's own keys, parsed once, next to the gate rules that
//! `path-gates` compiles from the same text.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use path_gates::Rules;
use serde::Deserialize;

/// Base ref used when neither `--base` nor the config names one.
pub const DEFAULT_BASE: &str = "origin/main";

// Unknown keys stay allowed: `paths`, `always` and `unmatched` belong to `path-gates`, which
// reads the same text.
#[derive(Deserialize)]
struct Raw {
    base: Option<String>,
    workspace: Option<PathBuf>,
    fallback: Option<String>,
    #[serde(default)]
    gate: Vec<RawGate>,
}

#[derive(Deserialize)]
struct RawGate {
    name: String,
    run: Option<String>,
}

/// One piece of a gate's `run` template.
pub enum Part {
    Text(String),
    Packages,
    NextestFilter,
    Changed,
}

/// A gate's name and command template.
pub struct Gate {
    pub name: String,
    pub run: Vec<Part>,
}

impl Gate {
    /// Whether the command needs the affected cargo packages (and so a `cargo metadata` run).
    pub fn needs_packages(&self) -> bool {
        self.run
            .iter()
            .any(|p| matches!(p, Part::Packages | Part::NextestFilter))
    }
}

/// The parsed configuration.
pub struct Config {
    /// Git top level of the config file's directory (the directory itself outside git):
    /// commands run here, and changed paths, globs and `workspace` are relative to it.
    pub dir: PathBuf,
    pub base: Option<String>,
    /// Cargo workspace root, relative to [`dir`](Self::dir); empty for the directory itself.
    pub workspace: PathBuf,
    /// `fallback = "all"`: a planning error runs every gate instead of failing. Off by
    /// default; see `main.rs` for why failing is the default.
    pub fallback_all: bool,
    pub rules: Rules,
    /// Gates in config order.
    pub gates: Vec<Gate>,
}

impl Config {
    /// Read and validate the file at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read config {}", path.display()))?;
        let mut cfg = Self::parse(&text, path)
            .with_context(|| format!("invalid config {}", path.display()))?;
        // `git-changed-paths` reports paths relative to the repository top, so a config kept
        // below it must still plan from the top. Outside git the config's directory stays;
        // planning then fails there with exit 3.
        if let Some(top) = toplevel(&cfg.dir) {
            cfg.dir = top;
        }
        Ok(cfg)
    }

    fn parse(text: &str, path: &Path) -> Result<Self> {
        let raw: Raw = toml::from_str(text)?;
        let rules = Rules::from_toml(text)?;
        let mut gates = Vec::new();
        for gate in raw.gate {
            let Some(run) = gate.run else {
                bail!("gate `{}` has no `run`", gate.name);
            };
            let run = parse_template(&run).with_context(|| format!("gate `{}`", gate.name))?;
            gates.push(Gate {
                name: gate.name,
                run,
            });
        }
        // `.` components are dropped so the empty path means "the repository top" and
        // `strip_prefix` works on it.
        let mut workspace = PathBuf::new();
        for component in raw.workspace.iter().flat_map(|w| w.components()) {
            match component {
                Component::CurDir => {}
                Component::Normal(part) => workspace.push(part),
                _ => bail!("`workspace` must be a relative path inside the repository"),
            }
        }
        let fallback_all = match raw.fallback.as_deref() {
            None | Some("error") => false,
            Some("all") => true,
            Some(other) => bail!("`fallback` must be \"error\" or \"all\", not \"{other}\""),
        };
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        Ok(Self {
            dir,
            base: raw.base,
            workspace,
            fallback_all,
            rules,
            gates,
        })
    }
}

/// Git top level of the repository holding `dir`; `None` outside git or without git.
pub fn toplevel(dir: &Path) -> Option<PathBuf> {
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim_end_matches('\n')))
}

// Only `{identifier}` is a placeholder. Other braces (`awk '{print $1}'`, `{a,b}`) and
// `${VAR}` stay literal so ordinary shell commands need no escaping.
fn parse_template(run: &str) -> Result<Vec<Part>> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut rest = run;
    while let Some(open) = rest.find('{') {
        let (head, tail) = rest.split_at(open);
        text.push_str(head);
        let name = tail[1..].split_once('}').map(|(name, _)| name);
        match name.filter(|n| is_identifier(n) && !text.ends_with('$')) {
            Some(name) => {
                let part = match name {
                    "packages" => Part::Packages,
                    "nextest_filter" => Part::NextestFilter,
                    "changed" => Part::Changed,
                    other => bail!("unknown placeholder `{{{other}}}`"),
                };
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(part);
                rest = &tail[name.len() + 2..];
            }
            None => {
                text.push('{');
                rest = &tail[1..];
            }
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    Ok(parts)
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
