// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! Agent Skills: `SKILL.md` discovery, the index line the model sees
//! up front, and the deferred `skill` tool that returns a body on demand.
//! The body stays out of the prompt until invoked — that is the whole point
//! of the format — so discovery and invocation are separate steps.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::frontmatter;

/// One parsed `SKILL.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    /// Tools the skill may use while active; empty means no restriction.
    pub allowed_tools: Vec<String>,
    pub metadata: BTreeMap<String, String>,
    pub compatibility: Option<String>,
    /// The `SKILL.md` path, for listings and error messages.
    pub path: PathBuf,
    /// Markdown after the frontmatter, returned on invoke.
    pub body: String,
}

#[derive(Debug, Default, PartialEq)]
pub struct Discovered {
    /// Search order, later directories replacing earlier same-name skills
    /// (project over home).
    pub skills: Vec<Skill>,
    pub notices: Vec<String>,
}

#[derive(Deserialize)]
struct Header {
    name: Option<String>,
    description: Option<String>,
    license: Option<String>,
    #[serde(rename = "allowed-tools")]
    allowed_tools: Option<Value>,
    metadata: Option<BTreeMap<String, Value>>,
    compatibility: Option<String>,
}

/// The directories to scan, in precedence order (later wins). `app_home` is
/// the host's own home directory and `app_dir` its per-project directory
/// name (`.cox`); the Claude Code locations are scanned after the host's own.
pub fn skill_dirs(
    app_dir: &str,
    app_home: Option<&Path>,
    claude_home: Option<&Path>,
    project: Option<&Path>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(h) = app_home {
        dirs.push(h.join("skills"));
    }
    if let Some(h) = claude_home {
        dirs.push(h.join("skills"));
    }
    if let Some(p) = project {
        dirs.push(p.join(app_dir).join("skills"));
        dirs.push(p.join(".claude").join("skills"));
    }
    dirs
}

/// Scans `<dir>/*/SKILL.md` for each directory. Malformed skills are
/// skipped with a notice; a missing directory is simply empty.
pub fn discover(dirs: &[PathBuf]) -> Discovered {
    let mut found = Discovered::default();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path().join("SKILL.md"))
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        for path in paths {
            match parse_skill(&path) {
                Ok(skill) => {
                    found.skills.retain(|s| s.name != skill.name);
                    found.skills.push(skill);
                }
                Err(reason) => found
                    .notices
                    .push(format!("skill {} skipped: {reason}", path.display())),
            }
        }
    }
    found
}

/// The spec's name rule: lowercase letters, digits and hyphens, ≤ 64 chars,
/// and equal to the directory name.
fn valid_name(name: &str, path: &Path) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("name must be 1–64 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "name `{name}` must be lowercase letters, digits and hyphens"
        ));
    }
    let dir = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if dir != name {
        return Err(format!(
            "name `{name}` does not match its directory `{dir}`"
        ));
    }
    Ok(())
}

fn parse_skill(path: &Path) -> Result<Skill, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (header, body): (Header, &str) = frontmatter::parse(&text).map_err(|e| e.to_string())?;
    let name = header.name.ok_or("missing `name`")?;
    valid_name(&name, path)?;
    let description = header
        .description
        .filter(|d| !d.trim().is_empty())
        .ok_or("missing `description`")?;
    let metadata = header
        .metadata
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| {
            let v = match v {
                Value::String(s) => s,
                other => serde_saphyr::to_string(&other)
                    .unwrap_or_default()
                    .trim_end()
                    .to_string(),
            };
            (k, v)
        })
        .collect();
    Ok(Skill {
        name,
        description: description.trim().to_string(),
        license: header.license,
        allowed_tools: frontmatter::names(header.allowed_tools.as_ref()),
        metadata,
        compatibility: header.compatibility,
        path: path.to_path_buf(),
        body: body.trim().to_string(),
    })
}

/// The `system[2]` index: one line per skill, no bodies. Empty when there
/// are no skills so the prefix does not change for users without any.
pub fn index(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("# Skills\nCall the `skill` tool with a name to load its instructions.\n");
    for s in skills {
        out.push_str(&format!("- {}: {}\n", s.name, s.description));
    }
    out
}

/// The deferred `skill` tool, host-neutral: [`SkillTool::spec`] is what to
/// register, [`SkillTool::call`] answers one invocation. The host wraps both
/// in its own tool trait (read-only, safe to run in parallel) and applies
/// `allowed_tools` to the engine.
pub struct SkillTool {
    skills: Arc<Vec<Skill>>,
}

/// What a host registers for the tool.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Loaded on demand rather than listed in every request.
    pub deferred: bool,
}

/// One invocation's answer: the body as visible text, plus the narrowing.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillOutput {
    pub text: String,
    pub name: String,
    pub allowed_tools: Vec<String>,
}

impl SkillOutput {
    /// `{name, allowed_tools}`, the shape hosts hand to their engine.
    pub fn structured(&self) -> Value {
        json!({
            "name": self.name,
            "allowed_tools": self.allowed_tools,
        })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("no such skill")]
pub struct SkillNotFound;

impl SkillTool {
    pub fn new(skills: Vec<Skill>) -> Self {
        Self {
            skills: Arc::new(skills),
        }
    }

    pub fn spec(&self) -> SkillToolSpec {
        SkillToolSpec {
            name: "skill".into(),
            description: "Load a skill's full instructions by name. The names and one-line descriptions are listed under `# Skills` in the system prompt.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Skill name from the index." } },
                "required": ["name"]
            }),
            deferred: true,
        }
    }

    /// The skill name, for permission prompts.
    pub fn subject(&self, input: &Value) -> String {
        input["name"].as_str().unwrap_or("").to_string()
    }

    /// `{"name": "<skill>"}` → the skill body. An unknown name is an error
    /// the host reports to the model; the body stays out of the prompt until
    /// this call.
    pub fn call(&self, input: &Value) -> Result<SkillOutput, SkillNotFound> {
        let name = input["name"].as_str().unwrap_or("");
        let skill = self
            .skills
            .iter()
            .find(|s| s.name == name)
            .ok_or(SkillNotFound)?;
        Ok(SkillOutput {
            text: format!("# Skill: {}\n\n{}", skill.name, skill.body),
            name: skill.name.clone(),
            allowed_tools: skill.allowed_tools.clone(),
        })
    }
}
