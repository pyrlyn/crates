// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! Permission rule grammar (Claude Code's `Tool(subject)`): one rule string
//! becomes a tool matcher plus a subject matcher. Separate from the engine
//! so the grammar is table-testable without a decision order around it.

use std::path::Path;

use globset::{Glob, GlobMatcher};

/// Which tool names exist for the grammar: aliases onto canonical names and
/// which tools take path globs. The default is Claude Code's set; a host
/// adds its own kinds (`App`, `Site`) with [`Grammar::alias`] and
/// [`Grammar::path_tool`]. A tool the grammar does not know still parses:
/// its subject is exact text, or a prefix, or a domain.
#[derive(Debug, Clone)]
pub struct Grammar {
    aliases: Vec<(String, String)>,
    path_tools: Vec<String>,
}

impl Default for Grammar {
    fn default() -> Self {
        let mut g = Self::empty();
        for (from, to) in [
            ("webfetch", "web_fetch"),
            ("websearch", "web_search"),
            ("multiedit", "edit"),
            ("notebookedit", "edit"),
        ] {
            g = g.alias(from, to);
        }
        for tool in [
            "read",
            "edit",
            "write",
            "grep",
            "glob",
            "outline",
            "apply_patch",
        ] {
            g = g.path_tool(tool);
        }
        g
    }
}

impl Grammar {
    /// No aliases and no path tools.
    pub fn empty() -> Self {
        Self {
            aliases: Vec::new(),
            path_tools: Vec::new(),
        }
    }

    /// Reads `from` as `to` in rules, calls and grants (both lower-cased).
    pub fn alias(mut self, from: &str, to: &str) -> Self {
        self.aliases
            .push((from.to_ascii_lowercase(), to.to_ascii_lowercase()));
        self
    }

    /// Makes `tool`'s rule subjects path globs.
    pub fn path_tool(mut self, tool: &str) -> Self {
        self.path_tools.push(tool.to_ascii_lowercase());
        self
    }

    /// The lower-cased, aliased name a rule or call is compared by.
    pub fn canonical(&self, name: &str) -> String {
        let lower = name.trim().to_ascii_lowercase();
        match self.aliases.iter().find(|(from, _)| *from == lower) {
            Some((_, to)) => to.clone(),
            None => lower,
        }
    }

    fn is_path_tool(&self, tool: &str) -> bool {
        self.path_tools.iter().any(|t| t == tool)
    }
}

/// The subject half of a rule (`Tool(subject)`).
#[derive(Debug, Clone)]
pub enum Subject {
    /// `Tool` — any subject.
    Any,
    /// `Tool(exact text)`.
    Exact(String),
    /// `Tool(prefix:*)` — matches `prefix` alone or `prefix` followed by whitespace.
    Prefix(String),
    /// `Tool(domain:example.com)` — the URL's host or a subdomain of it.
    Domain(String),
    /// A path glob for path tools; relative globs also match under `cwd`.
    Path(Vec<GlobMatcher>),
}

/// One compiled rule.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The rule as written, for ask reasons and deny messages.
    pub raw: String,
    /// The canonical tool name, or an `mcp__server__*`-style prefix.
    pub tool: String,
    /// What the subject must look like.
    pub subject: Subject,
}

impl Rule {
    /// Parses one rule. `home` expands a leading `~/`; `cwd` anchors
    /// relative path globs.
    pub fn parse(
        raw: &str,
        grammar: &Grammar,
        home: Option<&Path>,
        cwd: &Path,
    ) -> Result<Rule, String> {
        let raw = raw.trim();
        let (tool, inner) = match raw.split_once('(') {
            None => (raw, None),
            Some((tool, rest)) => (
                tool,
                Some(rest.strip_suffix(')').ok_or("missing closing ')'")?),
            ),
        };
        if tool.trim().is_empty() {
            return Err("empty tool name".into());
        }
        let tool = grammar.canonical(tool);
        let subject = match inner.map(str::trim) {
            None | Some("") => Subject::Any,
            Some(s) => {
                if let Some(prefix) = s.strip_suffix(":*") {
                    Subject::Prefix(prefix.trim_end().into())
                } else if let Some(domain) = s.strip_prefix("domain:") {
                    Subject::Domain(domain.trim().to_ascii_lowercase())
                } else if grammar.is_path_tool(&tool) {
                    Subject::Path(path_globs(s, home, cwd)?)
                } else {
                    Subject::Exact(s.into())
                }
            }
        };
        Ok(Rule {
            raw: raw.into(),
            tool,
            subject,
        })
    }

    /// Whether this rule covers `(tool, subject)`.
    pub fn matches(&self, grammar: &Grammar, tool: &str, subject: &str) -> bool {
        if !tool_matches(grammar, &self.tool, tool) {
            return false;
        }
        match &self.subject {
            Subject::Any => true,
            Subject::Exact(s) => s == subject,
            Subject::Prefix(p) => p.is_empty() || word_prefix(p, subject),
            Subject::Domain(d) => {
                host(subject).is_some_and(|h| h == *d || h.ends_with(&format!(".{d}")))
            }
            Subject::Path(globs) => globs.iter().any(|g| g.is_match(subject)),
        }
    }

    /// Whether this rule covers the whole command line on its own: only an
    /// `Any` or `Exact` rule may, since a prefix says nothing about what is
    /// chained after it.
    pub fn matches_line(&self, grammar: &Grammar, tool: &str, line: &str) -> bool {
        matches!(self.subject, Subject::Any | Subject::Exact(_))
            && self.matches(grammar, tool, line)
    }
}

/// `subject` is `prefix` alone or `prefix` followed by whitespace, so
/// `npm run test` covers `npm run test -- --watch` but not `npm run tests`.
pub(crate) fn word_prefix(prefix: &str, subject: &str) -> bool {
    subject
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

/// `mcp__server__*` matches by prefix; everything else by canonical name.
pub(crate) fn tool_matches(grammar: &Grammar, rule_tool: &str, call_tool: &str) -> bool {
    let call = grammar.canonical(call_tool);
    match rule_tool.strip_suffix('*') {
        Some(prefix) => call.starts_with(prefix),
        None => rule_tool == call,
    }
}

fn path_globs(pattern: &str, home: Option<&Path>, cwd: &Path) -> Result<Vec<GlobMatcher>, String> {
    let mut patterns = Vec::new();
    match (pattern.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => patterns.push(home.join(rest).to_string_lossy().into_owned()),
        (Some(_), None) => patterns.push(pattern.to_owned()),
        (None, _) if pattern.starts_with('/') => patterns.push(pattern.to_owned()),
        (None, _) => {
            patterns.push(pattern.to_owned());
            patterns.push(cwd.join(pattern).to_string_lossy().into_owned());
        }
    }
    patterns
        .iter()
        .map(|p| {
            Glob::new(p)
                .map(|g| g.compile_matcher())
                .map_err(|e| e.to_string())
        })
        .collect()
}

fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn rule(raw: &str) -> Rule {
        Rule::parse(
            raw,
            &Grammar::default(),
            Some(Path::new("/home/u")),
            Path::new("/repo"),
        )
        .expect("parses")
    }

    fn hit(raw: &str, tool: &str, subject: &str) -> bool {
        rule(raw).matches(&Grammar::default(), tool, subject)
    }

    #[test]
    fn rule_grammar_matches_claude_code_forms() {
        assert!(hit("Bash", "bash", "anything"));
        assert!(hit(
            "Bash(npm run test:*)",
            "bash",
            "npm run test -- --watch"
        ));
        assert!(hit("Bash(npm run test:*)", "bash", "npm run test"));
        assert!(!hit("Bash(npm run test:*)", "bash", "npm run tests"));
        assert!(hit("Bash(rm -rf /*)", "bash", "rm -rf /*"));
        assert!(!hit("Bash(rm -rf /*)", "bash", "rm -rf /tmp"));
        assert!(hit("Read(~/.ssh/**)", "read", "/home/u/.ssh/id_rsa"));
        assert!(hit("Edit(src/**)", "edit", "/repo/src/a.rs"));
        assert!(hit("Edit(src/**)", "edit", "src/a.rs"));
        assert!(!hit("Edit(src/**)", "write", "src/a.rs"));
        assert!(hit(
            "WebFetch(domain:example.com)",
            "web_fetch",
            "https://api.example.com/x"
        ));
        assert!(!hit(
            "WebFetch(domain:example.com)",
            "web_fetch",
            "https://example.com.evil/x"
        ));
        assert!(hit("mcp__gh__*", "mcp__gh__issues", ""));
        assert!(!hit("mcp__gh__*", "mcp__slack__post", ""));
        assert!(hit("mcp__gh__issues", "mcp__gh__issues", ""));
        assert!(hit("read", "Read", "/x"));
        let g = Grammar::default();
        assert!(Rule::parse("Bash(", &g, None, &PathBuf::from("/")).is_err());
        assert!(Rule::parse("", &g, None, &PathBuf::from("/")).is_err());
    }

    #[test]
    fn host_kinds_parse_without_changing_the_grammar() {
        assert!(hit("App(com.apple.Safari)", "app", "com.apple.Safari"));
        assert!(!hit(
            "App(com.apple.Safari)",
            "app",
            "com.apple.Safari.evil"
        ));
        assert!(hit("App(Visual Studio:*)", "app", "Visual Studio Code"));
        assert!(hit(
            "Site(domain:example.com)",
            "site",
            "https://a.example.com/x"
        ));
        assert!(!hit(
            "Site(domain:example.com)",
            "site",
            "https://example.com.evil/x"
        ));
        assert!(hit("Site", "SITE", "https://anything"));
    }

    #[test]
    fn a_host_grammar_adds_aliases_and_path_tools() {
        let g = Grammar::default().alias("Browse", "site").path_tool("file");
        let parse = |raw| Rule::parse(raw, &g, None, Path::new("/repo")).expect("parses");
        assert!(parse("Browse(https://a.test/x)").matches(&g, "site", "https://a.test/x"));
        assert!(parse("File(docs/**)").matches(&g, "file", "/repo/docs/a.md"));
        // Without the registration the same rule is exact text.
        assert!(!hit("File(docs/**)", "file", "/repo/docs/a.md"));
    }
}
