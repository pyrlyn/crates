// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! Skills from a vendored `anthropics/skills` sample plus a small own one —
//! the index carries names and descriptions only; the body arrives on
//! invoke, with `allowed-tools` alongside for the engine.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use agent_ext::skills::{SkillNotFound, SkillTool, discover, index, skill_dirs};
use serde_json::json;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skills")
}

#[test]
fn skills_index_lists_names_and_descriptions_without_bodies() {
    let found = discover(&[fixtures()]);
    assert!(found.notices.is_empty(), "{:?}", found.notices);
    let names: Vec<&str> = found.skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["greeting", "skill-creator"]);
    let text = index(&found.skills);
    assert!(text.starts_with("# Skills\n"), "{text}");
    assert!(
        text.contains("- skill-creator: Create new skills"),
        "{text}"
    );
    assert!(
        text.contains("- greeting: Greet the user in their language"),
        "{text}"
    );
    // Two skills, two index lines, no body content.
    assert_eq!(text.lines().filter(|l| l.starts_with("- ")).count(), 2);
    assert!(!text.contains("Say hello in the language"), "{text}");
    assert!(index(&[]).is_empty());
}

#[test]
fn skills_invoke_returns_the_body_and_allowed_tools() {
    let found = discover(&[fixtures()]);
    let tool = SkillTool::new(found.skills.clone());
    assert!(tool.spec().deferred);
    assert_eq!(tool.spec().name, "skill");
    assert_eq!(tool.subject(&json!({ "name": "greeting" })), "greeting");
    let out = tool.call(&json!({ "name": "greeting" })).unwrap();
    assert!(
        out.text.starts_with("# Skill: greeting\n\n# Greeting\n"),
        "{}",
        out.text
    );
    assert!(out.text.contains("Say hello in the language"));
    assert_eq!(out.structured()["allowed_tools"], json!(["read", "grep"]));

    let sample = tool.call(&json!({ "name": "skill-creator" })).unwrap();
    let vendored = std::fs::read_to_string(fixtures().join("skill-creator/SKILL.md")).unwrap();
    // The whole vendored body is there, not a summary.
    assert!(
        sample.text.len() > vendored.len() / 2,
        "{}",
        sample.text.len()
    );
    assert_eq!(tool.call(&json!({ "name": "nope" })), Err(SkillNotFound));
}

#[test]
fn skills_vendored_sample_parses_its_frontmatter() {
    let found = discover(&[fixtures()]);
    let sample = found
        .skills
        .iter()
        .find(|s| s.name == "skill-creator")
        .unwrap();
    assert!(sample.description.len() > 40);
    assert!(sample.allowed_tools.is_empty());
    let own = found.skills.iter().find(|s| s.name == "greeting").unwrap();
    assert_eq!(own.license.as_deref(), Some("MIT"));
    assert_eq!(own.metadata["author"], "cox");
    assert_eq!(own.metadata["version"], "1");
    assert!(
        own.compatibility
            .as_deref()
            .unwrap()
            .starts_with("Needs nothing")
    );
}

#[test]
fn skills_malformed_or_misnamed_are_skipped_with_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let mk = |name: &str, text: &str| {
        let d = dir.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), text).unwrap();
    };
    mk("no-front", "# Just markdown\n");
    mk("bad-yaml", "---\nname: [unclosed\n---\nbody\n");
    mk("mismatch", "---\nname: other\ndescription: d\n---\nbody\n");
    mk("no-desc", "---\nname: no-desc\n---\nbody\n");
    mk("Upper", "---\nname: Upper\ndescription: d\n---\nbody\n");
    mk("good", "---\nname: good\ndescription: fine\n---\nbody\n");
    let found = discover(&[dir.path().to_path_buf()]);
    let names: Vec<&str> = found.skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["good"]);
    assert_eq!(found.notices.len(), 5, "{:?}", found.notices);
    assert!(
        found
            .notices
            .iter()
            .all(|n| n.starts_with("skill ") && n.contains("skipped: "))
    );
}

#[test]
fn skills_later_directories_override_earlier_same_names() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for (root, desc) in [(home.path(), "from home"), (project.path(), "from project")] {
        let d = root.join(".claude/skills/dup");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("SKILL.md"),
            format!("---\nname: dup\ndescription: {desc}\n---\nbody\n"),
        )
        .unwrap();
    }
    let dirs = skill_dirs(
        ".cox",
        None,
        Some(&home.path().join(".claude")),
        Some(project.path()),
    );
    assert_eq!(dirs.len(), 3);
    let found = discover(&dirs);
    assert_eq!(found.skills.len(), 1);
    assert_eq!(found.skills[0].description, "from project");
}

#[test]
fn skills_metadata_non_string_values_render_as_yaml_text() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("meta");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("SKILL.md"),
        "---\nname: meta\ndescription: d\nmetadata:\n  quoted: \"1\"\n  int: 2\n  float: 1.5\n  flag: true\n  nothing: null\n  tags: [a, b]\n  nested: {k: v}\n---\nbody\n",
    )
    .unwrap();
    let found = discover(&[dir.path().to_path_buf()]);
    assert!(found.notices.is_empty(), "{:?}", found.notices);
    let m = &found.skills[0].metadata;
    let got: Vec<(&str, &str)> = m.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    // Pinned: the same text serde_yaml 0.9 gave before the move to serde-saphyr.
    assert_eq!(
        got,
        [
            ("flag", "true"),
            ("float", "1.5"),
            ("int", "2"),
            ("nested", "k: v"),
            ("nothing", "null"),
            ("quoted", "1"),
            ("tags", "- a\n- b"),
        ]
    );
}
