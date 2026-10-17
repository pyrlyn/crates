//! The JSON Schema of a config type, and the check that the committed copy
//! has not gone stale. The types are the source; the file only publishes them.

use std::fs;
use std::path::Path;

use schemars::generate::SchemaSettings;
use schemars::transform::RecursiveTransform;
use schemars::{JsonSchema, Schema};
use serde_json::Value;

use crate::error::{Error, Result};

/// The schema of `T`, pretty-printed with a trailing newline, in the form to
/// commit. `comment` becomes the root `$comment`, the place to say the file is
/// generated and how to regenerate it.
pub fn schema_text<T: JsonSchema>(comment: Option<&str>) -> Result<String> {
    let settings = SchemaSettings::draft2020_12()
        .with_transform(RecursiveTransform(drop_null as fn(&mut Schema)));
    let mut schema = settings.into_generator().into_root_schema_for::<T>();
    if let Some(comment) = comment {
        schema.insert("$comment".into(), comment.into());
    }
    let mut text = serde_json::to_string_pretty(&schema)?;
    text.push('\n');
    Ok(text)
}

/// Fails with [`Error::StaleSchema`] when the file at `path` is not what `T`
/// generates; the error names `bless_var`, the environment variable that makes
/// this call rewrite the file instead. Call it from a test so a type change
/// that forgets the schema fails CI.
pub fn check_schema<T: JsonSchema>(
    path: &Path,
    comment: Option<&str>,
    bless_var: &str,
) -> Result<()> {
    let bless = std::env::var_os(bless_var).is_some();
    check_with::<T>(path, comment, bless, bless_var)
}

fn check_with<T: JsonSchema>(
    path: &Path,
    comment: Option<&str>,
    bless: bool,
    bless_var: &str,
) -> Result<()> {
    let generated = schema_text::<T>(comment)?;
    let io = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    if bless {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        return fs::write(path, generated).map_err(io);
    }
    let committed = match fs::read_to_string(path) {
        // A Windows checkout may have turned LF into CRLF; the schema is the same.
        Ok(text) => text.replace("\r\n", "\n"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io(e)),
    };
    if committed == generated {
        return Ok(());
    }
    let line = committed
        .lines()
        .zip(generated.lines())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| committed.lines().count().min(generated.lines().count()))
        + 1;
    Err(Error::StaleSchema {
        path: path.to_path_buf(),
        detail: if committed.is_empty() {
            "missing or empty".to_string()
        } else {
            format!("first difference at line {line}")
        },
        bless_var: bless_var.to_string(),
    })
}

/// TOML has no null: an absent key is how a file says `None`, so `Option<T>`
/// is described as plain `T`, and an editor never offers `null` as a default.
/// Handles both shapes schemars emits, `type: [T, "null"]` and
/// `anyOf: [T, {type: null}]`.
fn drop_null(schema: &mut Schema) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };
    if let Some(default) = obj.get_mut("default") {
        strip_nulls(default);
        if default.is_null() {
            obj.remove("default");
        }
    }
    if let Some(Value::Array(types)) = obj.get_mut("type") {
        types.retain(|t| t != "null");
        if types.len() == 1 {
            let only = types.remove(0);
            obj.insert("type".into(), only);
        }
    }
    let Some(Value::Array(variants)) = obj.get_mut("anyOf") else {
        return;
    };
    let is_null = |v: &Value| v.get("type").is_some_and(|t| t == "null");
    if !variants.iter().any(is_null) {
        return;
    }
    variants.retain(|v| !is_null(v));
    if variants.len() == 1 {
        let only = variants.remove(0);
        obj.remove("anyOf");
        if let Value::Object(inner) = only {
            obj.extend(inner);
        }
    }
}

fn strip_nulls(value: &mut Value) {
    if let Value::Object(map) = value {
        map.retain(|_, v| !v.is_null());
        map.values_mut().for_each(strip_nulls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    // why: the fields exist only to be described by the generated schema.
    #[allow(dead_code)]
    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Sample {
        /// Where to listen.
        port: u16,
        note: Option<String>,
        mode: Mode,
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    enum Mode {
        Fast,
        Safe,
    }

    #[test]
    fn the_schema_carries_the_constraints_of_the_types() {
        let text = schema_text::<Sample>(None).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\"additionalProperties\": false"), "{text}");
        assert!(
            text.contains("\"fast\"") && text.contains("\"safe\""),
            "{text}"
        );
    }

    #[test]
    fn an_option_is_described_as_its_plain_type() {
        let schema: Value = serde_json::from_str(&schema_text::<Sample>(None).unwrap()).unwrap();
        let note = &schema["properties"]["note"];
        assert_eq!(note["type"], "string", "{note}");
        assert!(!schema.to_string().contains("null"), "{schema}");
    }

    #[test]
    fn a_null_default_is_dropped() {
        let mut schema: Schema = serde_json::from_value(
            serde_json::json!({"type": ["integer", "null"], "default": null}),
        )
        .unwrap();
        drop_null(&mut schema);
        assert_eq!(schema.to_value(), serde_json::json!({"type": "integer"}));
    }

    #[test]
    fn an_any_of_with_null_collapses_to_the_other_branch() {
        let mut schema: Schema = serde_json::from_value(serde_json::json!({
            "anyOf": [{"type": "string", "minLength": 1}, {"type": "null"}]
        }))
        .unwrap();
        drop_null(&mut schema);
        assert_eq!(
            schema.to_value(),
            serde_json::json!({"type": "string", "minLength": 1})
        );
    }

    #[test]
    fn the_comment_is_written_at_the_root() {
        let text = schema_text::<Sample>(Some("Generated. Do not edit.")).unwrap();
        assert!(
            text.contains("\"$comment\": \"Generated. Do not edit.\""),
            "{text}"
        );
    }

    #[test]
    fn a_current_file_passes_and_a_stale_one_fails_naming_the_variable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schema.json");
        let fresh = schema_text::<Sample>(None).unwrap();
        fs::write(&path, &fresh).unwrap();
        check_with::<Sample>(&path, None, false, "X_BLESS").unwrap();

        fs::write(&path, fresh.replace("Where to listen.", "Where we listen.")).unwrap();
        let err = check_with::<Sample>(&path, None, false, "X_BLESS").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("schema.json is stale"), "{message}");
        assert!(message.contains("first difference at line"), "{message}");
        assert!(message.contains("X_BLESS=1"), "{message}");
    }

    #[test]
    fn a_missing_file_is_stale_and_blessing_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("docs").join("schema.json");
        let err = check_with::<Sample>(&path, None, false, "X_BLESS").unwrap_err();
        assert!(err.to_string().contains("missing or empty"), "{err}");

        check_with::<Sample>(&path, None, true, "X_BLESS").unwrap();
        check_with::<Sample>(&path, None, false, "X_BLESS").unwrap();
    }

    #[test]
    fn crlf_line_endings_do_not_make_a_schema_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schema.json");
        let crlf = schema_text::<Sample>(None).unwrap().replace('\n', "\r\n");
        fs::write(&path, crlf).unwrap();
        check_with::<Sample>(&path, None, false, "X_BLESS").unwrap();
    }

    #[test]
    fn the_public_check_without_the_variable_set_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schema.json");
        let var = "CONFIG_SCHEMA_TEST_VARIABLE_NEVER_SET";
        assert!(check_schema::<Sample>(&path, None, var).is_err());
        assert!(!path.exists());
    }
}
