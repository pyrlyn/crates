//! The docs pipeline: `GUIDE.md` is the single source. It becomes the crate
//! docs (`#![doc = include_str!]`, so its examples run as doctests) and,
//! through this test, `llms.txt` and `llms-full.txt` next to the guide.
//! The test fails when those files are stale; `just docs` (which sets
//! `INJECTA_BLESS=1`) regenerates them.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const BLESS_VAR: &str = "INJECTA_BLESS";

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    crate_dir()
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))
}

/// The first paragraph after the `# injecta` title.
fn summary(guide: &str) -> String {
    guide
        .split("\n\n")
        .nth(1)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn headings(guide: &str) -> Vec<&str> {
    let mut in_code = false;
    guide
        .lines()
        .filter(|line| {
            if line.starts_with("```") {
                in_code = !in_code;
            }
            !in_code && line.starts_with("## ")
        })
        .map(|line| line.trim_start_matches("## "))
        .collect()
}

fn llms_txt(guide: &str) -> String {
    let mut out = format!(
        "# injecta\n\n> {}\n\nGenerated from GUIDE.md by `just docs`; do not edit.\n\n## Docs\n\n",
        summary(guide)
    );
    out.push_str("- [Guide](GUIDE.md): the user guide; also the crate-level rustdoc, every example is a doctest\n");
    out.push_str("- [Full text](llms-full.txt): the guide and the agent rules in one file\n");
    out.push_str("- [Agent rules](AGENTS.md): how to use and how to change injecta\n");
    out.push_str("- [Example](examples/web_app.rs): root container plus a per-request scope\n\n");
    out.push_str("## Guide sections\n\n");
    for heading in headings(guide) {
        let _ = writeln!(out, "- {heading}");
    }
    out
}

fn llms_full_txt(guide: &str, agents: &str) -> String {
    format!(
        "<!-- Generated from GUIDE.md and AGENTS.md by `just docs`; do not edit. -->\n\n{}\n\n---\n\n{}",
        guide.trim_end(),
        agents.trim_end()
    ) + "\n"
}

#[test]
fn llms_files_match_the_guide() -> TestResult {
    let root = repo_root();
    let agents_path = root.join("AGENTS.md");
    if !agents_path.exists() {
        // Running from a packaged crate: only the guide ships, nothing to compare.
        return Ok(());
    }
    let guide = read(&crate_dir().join("GUIDE.md"))?;
    let agents = read(&agents_path)?;
    let expected = [
        (root.join("llms.txt"), llms_txt(&guide)),
        (root.join("llms-full.txt"), llms_full_txt(&guide, &agents)),
    ];
    let bless = std::env::var_os(BLESS_VAR).is_some();
    for (path, content) in expected {
        if bless {
            std::fs::write(&path, content)
                .map_err(|err| format!("write {}: {err}", path.display()))?;
        } else {
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                current == content,
                "{} is stale: run `just docs`",
                path.display()
            );
        }
    }
    Ok(())
}

#[test]
fn guide_has_the_decision_table_and_anti_patterns() -> TestResult {
    let guide = read(&crate_dir().join("GUIDE.md"))?;
    let sections = headings(&guide);
    for required in [
        "The four lifetimes",
        "Which tool to use",
        "Anti-patterns",
        "Extension points",
    ] {
        assert!(
            sections.contains(&required),
            "GUIDE.md lost the `{required}` section"
        );
    }
    Ok(())
}
