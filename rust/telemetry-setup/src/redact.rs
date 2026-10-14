// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later

//! Secret masking shared by every log sink.
//!
//! Redaction works on the finished line, not on tracing fields, so one rule set covers events,
//! span fields, JSON and the human format alike, and a field added later cannot bypass it.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::Error;

/// What a masked value is replaced with.
pub const REDACTED: &str = "[REDACTED]";

/// Substrings that mark a field or `name=value` pair as secret, matched case-insensitively.
/// Matching a substring rather than a whole word over-masks `cache_key`, which is the safe side.
const SECRET_NAME_PARTS: &[&str] = &[
    "token",
    "key",
    "password",
    "passwd",
    "secret",
    "authorization",
    "credential",
    "cookie",
];

/// Credential shapes that are masked wherever they appear, even under an innocent field name:
/// bearer headers, provider API keys, forge tokens, chat tokens, cloud keys and JWTs.
const VALUE_PATTERNS: &[&str] = &[
    r"\bbearer\s+[A-Za-z0-9._~+/=-]{8,}",
    r"\b(?:sk|pk|rk)-[A-Za-z0-9_-]{16,}",
    r"\bgh[pousr]_[A-Za-z0-9]{20,}",
    r"\bgithub_pat_[A-Za-z0-9_]{20,}",
    r"\bxox[abprs]-[A-Za-z0-9-]{10,}",
    r"\bAKIA[0-9A-Z]{16}\b",
    r"\bAIza[0-9A-Za-z_-]{35}",
    r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
];

/// Keeps the name and separator, masks the value.
const PAIR_REPLACEMENT: &str = "${1}${2}[REDACTED]";

/// A compiled rule set: the built-in credential shapes plus any the application adds.
#[derive(Debug, Clone)]
pub struct Redactor {
    value: Regex,
    pair: Regex,
}

impl Redactor {
    /// The built-in rules plus `extra` credential shapes (regex syntax, matched
    /// case-insensitively), such as an application's own API token prefix.
    ///
    /// # Errors
    /// [`Error::Pattern`] naming the first pattern that does not compile.
    pub fn new<S: AsRef<str>>(extra: &[S]) -> Result<Self, Error> {
        let mut alternatives: Vec<&str> = VALUE_PATTERNS.to_vec();
        for pattern in extra {
            let pattern = pattern.as_ref();
            // Compiled alone first so the error names the caller's pattern, not the joined one.
            Regex::new(pattern).map_err(|source| Error::Pattern {
                pattern: pattern.to_owned(),
                source,
            })?;
            alternatives.push(pattern);
        }
        let value = format!("(?i)(?:(?:{}))", alternatives.join(")|(?:"));
        // `name=value` or `name: value` where the name looks secret; the value is quoted or one token.
        let pair = format!(
            r#"(?i)\b([\w.-]*(?:{})[\w.-]*)(\s*[=:]\s*)("(?:[^"\\]|\\.)*"|'[^']*'|[^\s,;}}]+)"#,
            SECRET_NAME_PARTS.join("|")
        );
        Ok(Self {
            value: compile(value)?,
            pair: compile(pair)?,
        })
    }

    /// `text` with credential-shaped values and the values of secret-named pairs masked.
    #[must_use]
    pub fn scrub_text<'a>(&self, text: &'a str) -> Cow<'a, str> {
        // Values first, so `Authorization: Bearer abc` is already one token when the pair rule runs.
        let after_values = self.value.replace_all(text, REDACTED);
        if let Cow::Owned(masked) = self.pair.replace_all(&after_values, PAIR_REPLACEMENT) {
            return Cow::Owned(masked);
        }
        after_values
    }

    /// Masks a parsed JSON log record in place: secret-named keys lose their whole value (of any
    /// type), every other string goes through [`Redactor::scrub_text`].
    pub fn scrub_json(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, inner) in map.iter_mut() {
                    if is_secret_name(key) {
                        *inner = Value::String(REDACTED.to_owned());
                    } else {
                        self.scrub_json(inner);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.scrub_json(item)),
            Value::String(text) => {
                if let Cow::Owned(masked) = self.scrub_text(text) {
                    *text = masked;
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// One log line without its newline: a JSON object is parsed and masked structurally,
    /// anything else (the human stderr format) is masked as text.
    #[must_use]
    pub fn scrub_line(&self, line: &str) -> String {
        match serde_json::from_str::<Value>(line) {
            Ok(mut record @ Value::Object(_)) => {
                self.scrub_json(&mut record);
                record.to_string()
            }
            _ => self.scrub_text(line).into_owned(),
        }
    }
}

fn compile(pattern: String) -> Result<Regex, Error> {
    Regex::new(&pattern).map_err(|source| Error::Pattern { pattern, source })
}

// A built-in pattern that fails to compile must not turn into a panic or a silent pass-through,
// so the free functions below mask everything instead. A test pins that the rules compile.
static STANDARD: LazyLock<Option<Redactor>> = LazyLock::new(|| Redactor::new::<&str>(&[]).ok());

/// Whether a field or key name marks its value as secret.
#[must_use]
pub fn is_secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SECRET_NAME_PARTS.iter().any(|part| name.contains(part))
}

/// [`Redactor::scrub_text`] with the built-in rules. Fails closed: if they are unavailable, the
/// whole text is masked.
#[must_use]
pub fn scrub_text(text: &str) -> Cow<'_, str> {
    match STANDARD.as_ref() {
        Some(redactor) => redactor.scrub_text(text),
        None => Cow::Borrowed(REDACTED),
    }
}

/// [`Redactor::scrub_json`] with the built-in rules. Fails closed like [`scrub_text`].
pub fn scrub_json(value: &mut Value) {
    match STANDARD.as_ref() {
        Some(redactor) => redactor.scrub_json(value),
        None => *value = Value::String(REDACTED.to_owned()),
    }
}

/// [`Redactor::scrub_line`] with the built-in rules. Fails closed like [`scrub_text`].
#[must_use]
pub fn scrub_line(line: &str) -> String {
    match STANDARD.as_ref() {
        Some(redactor) => redactor.scrub_line(line),
        None => REDACTED.to_owned(),
    }
}
