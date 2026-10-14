// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! YAML frontmatter (`---` … `---`) shared by skills and any other
//! extension file with a header. One place to split and parse so every
//! extension file fails the same way: a malformed header is an error the
//! caller turns into a notice, never a panic and never a silently empty
//! definition.

use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FrontmatterError {
    #[error("no frontmatter block (expected `---` on the first line)")]
    Missing,
    #[error("unterminated frontmatter (no closing `---`)")]
    Unterminated,
    #[error("frontmatter: {0}")]
    Yaml(String),
}

/// Splits `text` into its YAML header and the body after the closing `---`.
pub fn split(text: &str) -> Result<(&str, &str), FrontmatterError> {
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))
        .ok_or(FrontmatterError::Missing)?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Ok((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err(FrontmatterError::Unterminated)
}

/// Parses the header into `T` and returns it with the body.
pub fn parse<T: DeserializeOwned>(text: &str) -> Result<(T, &str), FrontmatterError> {
    let (yaml, body) = split(text)?;
    let value = serde_saphyr::from_str(yaml).map_err(|e| FrontmatterError::Yaml(e.to_string()))?;
    Ok((value, body))
}

/// A field that Claude writes either as a YAML list or as one string of
/// space- or comma-separated names (`allowed-tools: Read Bash`).
pub fn names(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::trim).map(String::from))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(serde_json::Value::String(s)) => s
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_splits_header_from_body() {
        let (yaml, body) = split("---\nname: x\n---\nbody\n").unwrap();
        assert_eq!(yaml, "name: x\n");
        assert_eq!(body, "body\n");
        assert_eq!(split("no header"), Err(FrontmatterError::Missing));
        assert_eq!(split("---\nname: x\n"), Err(FrontmatterError::Unterminated));
    }

    #[test]
    fn frontmatter_names_accepts_list_and_string_forms() {
        let list: serde_json::Value = serde_saphyr::from_str("- read\n- grep\n").unwrap();
        assert_eq!(names(Some(&list)), ["read", "grep"]);
        let text = serde_json::Value::String("Read, Bash grep".into());
        assert_eq!(names(Some(&text)), ["Read", "Bash", "grep"]);
        assert!(names(None).is_empty());
    }
}
