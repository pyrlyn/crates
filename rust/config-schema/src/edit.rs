//! Changing keys of a TOML file in place with `toml_edit`: comments, blank
//! lines, key order and a key's trailing comment survive; only the value moves.
//!
//! An edit is planned first ([`plan_edit`]) and written only by
//! [`Edit::apply`], so a dry run is the same call without the write, and the
//! `before`/`after` texts can feed a diff.

use std::fs;
use std::path::{Path, PathBuf};

use serde::de::{DeserializeOwned, IntoDeserializer};
use toml_edit::{DocumentMut, Item, TableLike};

use crate::error::{Error, Result};
use crate::reveal::reveal_commented_key;

pub use toml_edit::Value as TomlValue;

/// A planned change to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub path: PathBuf,
    /// The file as it is now; empty when it does not exist yet.
    pub before: String,
    /// The file as it would be after the edit.
    pub after: String,
}

impl Edit {
    /// Whether the edit changes anything: setting a key to the value it
    /// already has leaves the text byte-for-byte as it was.
    pub fn changed(&self) -> bool {
        self.before != self.after
    }

    /// Writes `after` atomically, creating the parent directory; does nothing
    /// when the edit changes nothing.
    pub fn apply(&self) -> Result<()> {
        if !self.changed() {
            return Ok(());
        }
        write_atomic(&self.path, &self.after)
    }
}

/// Plans setting each `(dotted key, value)` in `path`, for a file this project
/// owns. Commented-out defaults on the key's path are revealed first so the
/// edit lands on their line. The result must still deserialize as `T`
/// (unknown keys, wrong types and out-of-range values live in the types), or
/// the edit is refused and the file is never written in a state the loader
/// would reject.
pub fn plan_edit<T: DeserializeOwned>(path: &Path, edits: &[(&str, TomlValue)]) -> Result<Edit> {
    let before = read_or_empty(path)?;
    let mut source = before.clone();
    for (key, _) in edits {
        source = reveal_commented_key(&source, key);
    }
    let after = edit_text(path, &source, edits)?;
    toml_edit::de::from_str::<T>(&after).map_err(|e| Error::Invalid {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(Edit {
        path: path.to_path_buf(),
        before,
        after,
    })
}

/// [`plan_edit`] for a file another program owns: no types, no validation and
/// no revealing of comments. Only the named keys change, so everything else
/// stays byte-for-byte what it was.
pub fn plan_edit_foreign(path: &Path, edits: &[(&str, TomlValue)]) -> Result<Edit> {
    let before = read_or_empty(path)?;
    let after = edit_text(path, &before, edits)?;
    Ok(Edit {
        path: path.to_path_buf(),
        before,
        after,
    })
}

/// Our own entry of a foreign file, at the dotted `key` (for example
/// `mcp_servers.ours`), checked as `E`. `Ok(None)` when it is not there; the
/// rest of the file is never looked at, so a foreign key we do not know is
/// never an error.
pub fn read_entry<E: DeserializeOwned>(path: &Path, text: &str, key: &str) -> Result<Option<E>> {
    let doc = parse_document(path, text)?;
    let mut item = doc.as_item();
    for part in key.split('.') {
        match item.get(part) {
            Some(next) => item = next,
            None => return Ok(None),
        }
    }
    let invalid = |message: String| Error::Invalid {
        path: path.to_path_buf(),
        message: format!("{key}: {message}"),
    };
    let value = item
        .clone()
        .into_value()
        .map_err(|_| invalid("expected a value or a table".to_string()))?;
    E::deserialize(value.into_deserializer())
        .map(Some)
        .map_err(|e| invalid(e.to_string()))
}

/// Parses `raw` as a TOML value (`5`, `true`, `"text"`, `[1, 2]`), falling
/// back to a bare string for input that is not valid TOML on its own, so
/// `set tier.model sonnet` needs no quotes.
pub fn parse_value(raw: &str) -> TomlValue {
    raw.parse()
        .unwrap_or_else(|_| TomlValue::from(raw.to_string()))
}

/// A JSON scalar or array as a TOML value, for a caller whose values arrive
/// typed. `null` and objects have no form a leaf key takes.
pub fn value_from_json(key: &str, value: &serde_json::Value) -> Result<TomlValue> {
    json_to_toml(value).ok_or_else(|| Error::UnsupportedValue {
        key: key.to_string(),
        value: value.to_string(),
    })
}

fn json_to_toml(value: &serde_json::Value) -> Option<TomlValue> {
    use serde_json::Value as Json;
    Some(match value {
        Json::Bool(b) => TomlValue::from(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => TomlValue::from(i),
            None => TomlValue::from(n.as_f64()?),
        },
        Json::String(s) => TomlValue::from(s.as_str()),
        Json::Array(items) => {
            let items: Option<Vec<TomlValue>> = items.iter().map(json_to_toml).collect();
            TomlValue::Array(items?.into_iter().collect())
        }
        Json::Null | Json::Object(_) => return None,
    })
}

fn read_or_empty(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn parse_document(path: &Path, text: &str) -> Result<DocumentMut> {
    text.parse()
        .map_err(|e: toml_edit::TomlError| Error::Parse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
}

fn edit_text(path: &Path, text: &str, edits: &[(&str, TomlValue)]) -> Result<String> {
    for (key, _) in edits {
        if key.split('.').any(str::is_empty) {
            return Err(Error::EmptyKey);
        }
    }
    let mut doc = parse_document(path, text)?;
    for (key, value) in edits {
        assign(&mut doc, key, value.clone())?;
    }
    Ok(doc.to_string())
}

/// Walks the dotted key, creating tables on the way. The walk is explicit
/// because `toml_edit`'s index operators panic on a path that runs through a
/// scalar; here it is an error naming the clash.
fn assign(doc: &mut DocumentMut, key: &str, value: TomlValue) -> Result<()> {
    let mut parts = key.split('.').peekable();
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            // `insert` replaces the whole (key, item) pair, dropping the key's
            // padding and its trailing comment; swap the value in place.
            match table.get_mut(part).and_then(Item::as_value_mut) {
                Some(old) => {
                    let decor = old.decor().clone();
                    *old = value;
                    *old.decor_mut() = decor;
                }
                None => {
                    table.insert(part, toml_edit::value(value));
                }
            }
            return Ok(());
        }
        if !table.contains_key(part) {
            table.insert(part, toml_edit::table());
        }
        table = table
            .get_mut(part)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| Error::NotATable {
                part: part.to_string(),
                key: key.to_string(),
            })?;
    }
    Err(Error::EmptyKey)
}

/// Writes `text` to `path` through a sibling temp file and a rename, so a kill
/// or a full disk never leaves a truncated config that every later run fails
/// to parse. A symlinked config (a dotfiles checkout) is followed to its
/// target, and an existing file keeps its permissions.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let io = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let temp = target.with_file_name(name);
    let write = || -> std::io::Result<()> {
        // Sync through the handle that wrote: Windows refuses `sync_all` on a read-only handle.
        let mut file = fs::File::create(&temp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        if let Ok(meta) = fs::metadata(&target) {
            fs::set_permissions(&temp, meta.permissions())?;
        }
        fs::rename(&temp, &target)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&temp);
        io(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Config {
        port: u16,
        mode: Mode,
        proxy: Proxy,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Proxy {
        upstream: String,
    }

    #[derive(Debug, Deserialize, Default, PartialEq)]
    #[serde(rename_all = "snake_case")]
    enum Mode {
        #[default]
        Fast,
        Safe,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                port: 1,
                mode: Mode::Fast,
                proxy: Proxy {
                    upstream: String::new(),
                },
            }
        }
    }

    fn file(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, text).unwrap();
        (dir, path)
    }

    fn set(path: &Path, key: &str, raw: &str) -> Result<Edit> {
        plan_edit::<Config>(path, &[(key, parse_value(raw))])
    }

    #[test]
    fn a_set_keeps_comments_and_layout_around_the_key() {
        let text = "# my notes\n\n[proxy]\n# pinned deliberately\nupstream = \"old\"   # keep me\n";
        let (_dir, path) = file(text);
        let edit = set(&path, "proxy.upstream", "\"new\"").unwrap();
        assert_eq!(
            edit.after,
            "# my notes\n\n[proxy]\n# pinned deliberately\nupstream = \"new\"   # keep me\n"
        );
    }

    #[test]
    fn an_unquoted_word_is_a_string() {
        let (_dir, path) = file("");
        assert_eq!(
            set(&path, "mode", "safe").unwrap().after,
            "mode = \"safe\"\n"
        );
        assert_eq!(parse_value("5").as_integer(), Some(5));
    }

    #[test]
    fn a_missing_file_plans_a_creation_that_apply_writes_with_its_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home").join("config.toml");
        let edit = set(&path, "port", "9").unwrap();
        assert_eq!(edit.before, "");
        assert!(!path.exists(), "planning must not write");
        edit.apply().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "port = 9\n");
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no temp file is left behind");
    }

    #[test]
    fn setting_the_current_value_changes_nothing() {
        let (_dir, path) = file("port = 9 # same\n");
        let edit = set(&path, "port", "9").unwrap();
        assert!(!edit.changed());
        edit.apply().unwrap();
    }

    #[test]
    fn a_key_below_a_scalar_is_an_error_not_a_panic() {
        let (_dir, path) = file("port = 9\n");
        let err = set(&path, "port.foo", "1").unwrap_err();
        assert!(matches!(err, Error::NotATable { .. }), "{err}");
        assert!(err.to_string().contains("`port` in `port.foo`"), "{err}");
    }

    #[test]
    fn an_empty_key_part_is_rejected() {
        let (_dir, path) = file("");
        for key in ["", "a..b", ".a", "a."] {
            assert!(
                matches!(set(&path, key, "1"), Err(Error::EmptyKey)),
                "{key:?}"
            );
        }
    }

    #[test]
    fn an_inline_table_is_a_table() {
        let (_dir, path) = file("proxy = { upstream = \"a\" }\n");
        let edit = set(&path, "proxy.upstream", "\"b\"").unwrap();
        assert_eq!(edit.after, "proxy = { upstream = \"b\" }\n");
    }

    #[test]
    fn a_result_the_types_reject_is_refused() {
        let (_dir, path) = file("port = 1\n");
        for (key, raw) in [
            ("prot", "1"),
            ("port", "\"eighty\""),
            ("port", "70000"),
            ("mode", "turbo"),
        ] {
            let err = set(&path, key, raw).unwrap_err();
            assert!(matches!(err, Error::Invalid { .. }), "{key}={raw}: {err}");
            assert!(err.to_string().contains("config.toml"), "{err}");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "port = 1\n");
    }

    #[test]
    fn invalid_toml_names_the_file() {
        let (_dir, path) = file("port = \n");
        let err = set(&path, "port", "1").unwrap_err();
        assert!(matches!(err, Error::Parse { .. }), "{err}");
        assert!(err.to_string().contains("config.toml"), "{err}");
    }

    #[test]
    fn a_commented_default_is_revealed_and_edited_in_place() {
        let text = "# port = 1  # the listen port\n[proxy]\n# upstream = \"\"\n";
        let (_dir, path) = file(text);
        let edit = plan_edit::<Config>(
            &path,
            &[
                ("port", parse_value("5")),
                ("proxy.upstream", parse_value("u")),
            ],
        )
        .unwrap();
        assert_eq!(
            edit.after,
            "port = 5  # the listen port\n[proxy]\nupstream = \"u\"\n"
        );
    }

    #[test]
    fn a_key_that_is_already_set_is_not_duplicated_by_its_commented_twin() {
        let text = "# port = 1\nport = 2\n";
        let (_dir, path) = file(text);
        let edit = set(&path, "port", "3").unwrap();
        assert_eq!(edit.after, "# port = 1\nport = 3\n");
    }

    #[test]
    fn json_values_become_typed_toml_and_null_or_objects_are_unsupported() {
        let (_dir, path) = file("");
        let edits = [
            (
                "port",
                value_from_json("port", &serde_json::json!(7)).unwrap(),
            ),
            (
                "mode",
                value_from_json("mode", &serde_json::json!("safe")).unwrap(),
            ),
        ];
        assert_eq!(
            plan_edit::<Config>(&path, &edits).unwrap().after,
            "port = 7\nmode = \"safe\"\n"
        );
        assert_eq!(
            value_from_json("k", &serde_json::json!([1.5, "a"]))
                .unwrap()
                .to_string(),
            "[1.5, \"a\"]"
        );
        for bad in [serde_json::json!(null), serde_json::json!({"a": 1})] {
            let err = value_from_json("k", &bad).unwrap_err();
            assert!(matches!(err, Error::UnsupportedValue { .. }), "{err}");
        }
    }

    #[test]
    fn a_foreign_edit_touches_only_the_named_key_and_validates_nothing() {
        let text = "# the host's own file\nmodel = \"x\"\nunknown_to_us = [1,   2]\n\n[mcp_servers.ours]\ncommand = \"old\" # ours\n";
        let (_dir, path) = file(text);
        let edit = plan_edit_foreign(
            &path,
            &[("mcp_servers.ours.command", parse_value("\"new\""))],
        )
        .unwrap();
        assert_eq!(edit.after, text.replace("\"old\"", "\"new\""));
    }

    #[test]
    fn our_entry_in_a_foreign_file_is_checked_and_the_rest_ignored() {
        #[derive(Debug, Deserialize, PartialEq)]
        #[serde(deny_unknown_fields)]
        struct Entry {
            command: String,
        }
        let path = Path::new("host.toml");
        let text = "weird = { x = 1 }\n[mcp_servers.ours]\ncommand = \"c\"\n[mcp_servers.theirs]\nfoo = 1\n";
        let entry: Option<Entry> = read_entry(path, text, "mcp_servers.ours").unwrap();
        assert_eq!(
            entry,
            Some(Entry {
                command: "c".into()
            })
        );
        assert_eq!(
            read_entry::<Entry>(path, text, "mcp_servers.absent").unwrap(),
            None
        );

        let err = read_entry::<Entry>(path, text, "mcp_servers.theirs").unwrap_err();
        assert!(err.to_string().contains("host.toml"), "{err}");
        assert!(err.to_string().contains("mcp_servers.theirs"), "{err}");
        let err = read_entry::<Entry>(path, "x = ", "x").unwrap_err();
        assert!(matches!(err, Error::Parse { .. }), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn apply_follows_a_symlink_and_keeps_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles.toml");
        fs::write(&real, "port = 1\n").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("config.toml");
        symlink(&real, &link).unwrap();

        set(&link, "port", "2").unwrap().apply().unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "port = 2\n");
        assert_eq!(
            fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
