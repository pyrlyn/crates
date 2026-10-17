// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! Register and unregister an MCP server in an agent host's JSON config file.
//!
//! An agent host (Claude Code `~/.claude.json`, Cursor `~/.cursor/mcp.json`, ...) owns its
//! config file; this crate changes only the one entry the caller names. The contract:
//!
//! - **Only our entry.** The file is edited as text through a lossless syntax tree, so every
//!   other key keeps its value, position and spelling, and comments, indentation and number
//!   formats survive. The one side effect is on the object that gains our entry: it is made
//!   multi-line, and its previous last entry gains a comma.
//! - **Never clobbers.** A file that is not JSON (comments and trailing commas are fine), or a
//!   root or server map that is not an object, is an error naming the file; it is never
//!   replaced.
//! - **Atomic.** The new body lands in a sibling temp file and is renamed over the target, so a
//!   crash never leaves a half-written config. A symlinked config keeps its link, and the file's
//!   permissions survive.
//! - **Reversible.** With [`Apply::backup`], the old file is copied to `<name>.bak-<unix-seconds>`
//!   first (through `file-backup`).
//! - **Idempotent.** A second apply returns [`NO_CHANGES`] and writes nothing.
//! - **Dry-runnable.** [`Apply::dry_run`] returns the same report and touches nothing.
//!
//! The report is also the write gate: [`write`] refuses to write on [`NO_CHANGES`], so "we said
//! nothing changed" and "we changed nothing" cannot drift apart.
//!
//! ```
//! use agent_host_config::{Apply, NO_CHANGES, register_mcp};
//! let dir = std::env::temp_dir().join(format!("agent-host-config-doc-{}", std::process::id()));
//! std::fs::create_dir_all(&dir)?;
//! let path = dir.join("mcp.json");
//! let apply = Apply { dry_run: false, backup: false };
//! let first = register_mcp(&apply, &path, "demo", "demo-bin", &["mcp"])?;
//! assert_eq!(first, "mcpServers.demo: demo-bin mcp");
//! assert_eq!(register_mcp(&apply, &path, "demo", "demo-bin", &["mcp"])?, NO_CHANGES);
//! std::fs::remove_dir_all(&dir).ok();
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use serde_json::{Value, json};

/// The report an edit returns when it found nothing to do. Also the write gate: a run carrying
/// this report never touches the disk.
pub const NO_CHANGES: &str = "no changes";

/// How much an edit is allowed to do to the disk on this run.
#[derive(Clone, Copy, Debug, Default)]
pub struct Apply {
    /// Describe the change and write nothing.
    pub dry_run: bool,
    /// Copy the file to `<name>.bak-<unix-seconds>` before the first write to it.
    pub backup: bool,
}

impl Apply {
    /// True when this run may write a file for `report`.
    pub fn writes(&self, report: &str) -> bool {
        !self.dry_run && report != NO_CHANGES
    }
}

/// Register a stdio MCP server under `mcpServers.<name>` in a host's JSON config — the shape
/// Claude Code (`~/.claude.json`) and Cursor (`~/.cursor/mcp.json`) both read.
pub fn register_mcp(
    apply: &Apply,
    path: &Path,
    name: &str,
    command: &str,
    args: &[&str],
) -> Result<String> {
    let summary = format!("{command} {}", args.join(" "));
    register_server(
        apply,
        path,
        "mcpServers",
        name,
        mcp_entry(command, args),
        &summary,
    )
}

/// The `mcpServers.<name>` entry [`register_mcp`] writes.
pub fn mcp_entry(command: &str, args: &[&str]) -> Value {
    json!({"type": "stdio", "command": command, "args": args})
}

/// [`register_mcp`] for a host whose server map or entry has another shape: OpenCode keeps
/// `mcp.<name> = {type: "local", command: [..]}`, Copilot adds `tools` to `mcpServers`.
/// `summary` is what the report prints after `<key>.<name>: `. A dotted `key` walks nested
/// objects (ZCode's `mcp.servers`), creating the missing ones.
pub fn register_server(
    apply: &Apply,
    path: &Path,
    key: &str,
    name: &str,
    entry: Value,
    summary: &str,
) -> Result<String> {
    let root = parse(path)?;
    let have = servers(&root, key).and_then(|s| s.get(name)?.to_serde_value());
    if have.as_ref().map(without_default_type) == Some(without_default_type(&entry)) {
        return Ok(NO_CHANGES.into());
    }
    let mut map = root
        .object_value_or_create()
        .with_context(|| format!("{}: the root is not an object", path.display()))?;
    for k in key.split('.') {
        map = map
            .object_value_or_create(k)
            .with_context(|| format!("{}: {k} is not an object", path.display()))?;
    }
    match map.get(name) {
        Some(prop) => prop.set_value(to_cst(&entry)),
        None => {
            map.append(name, to_cst(&entry));
        }
    }
    let report = format!("{key}.{name}: {summary}");
    write(apply, path, &root.to_string(), &report)?;
    Ok(report)
}

/// The entry at `<key>.<name>` (dotted `key` as in [`register_server`]), or `None` when the
/// file, the map or the entry is absent. For the ownership check [`unregister_server`] leaves to
/// its caller.
pub fn entry_at(path: &Path, key: &str, name: &str) -> Result<Option<Value>> {
    let root = parse(path)?;
    Ok(servers(&root, key).and_then(|s| s.get(name)?.to_serde_value()))
}

/// Drop the `<name>` entry from a host's `mcpServers` map. Foreign servers are left alone, and a
/// map that ends up empty goes with it so the file reads as it did before we arrived.
pub fn unregister_mcp(apply: &Apply, path: &Path, name: &str) -> Result<String> {
    unregister_server(apply, path, "mcpServers", name)
}

/// [`unregister_mcp`] under another map key (OpenCode's `mcp`), dotted for a nested one
/// (ZCode's `mcp.servers`); only the last level is dropped when it ends up empty.
///
/// It drops an entry by name alone, with no ownership check: a caller whose host can hold a
/// foreign server under the same name must first judge [`entry_at`] itself.
pub fn unregister_server(apply: &Apply, path: &Path, key: &str, name: &str) -> Result<String> {
    let root = parse(path)?;
    let Some(map) = servers(&root, key) else {
        return Ok(NO_CHANGES.into());
    };
    let Some(prop) = map.get(name) else {
        return Ok(NO_CHANGES.into());
    };
    prop.remove();
    if map.properties().is_empty()
        && let Some(prop) = parent_of_last(&root, key)
    {
        prop.remove();
    }
    let report = format!("- {key}.{name}");
    write(apply, path, &root.to_string(), &report)?;
    Ok(report)
}

/// The syntax tree of the file at `path`. A missing or blank file is an empty document
/// (setting up a host the user has never configured is the common case). Comments and trailing commas parse; anything else that is not JSON is an error
/// naming the file, the line and the column, so the entry can be pasted in by hand.
fn parse(path: &Path) -> Result<CstRootNode> {
    let mut text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| path.display().to_string()),
    };
    if text.trim().is_empty() {
        text.clear();
    }
    let options = ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    };
    CstRootNode::parse(&text, &options).map_err(|e| {
        anyhow::anyhow!(
            "{}: not JSON: {e}. Refusing to rewrite this file; add the entry by hand and re-run.",
            path.display()
        )
    })
}

/// The server map at the dotted `key`, when every level exists and is an object.
fn servers(root: &CstRootNode, key: &str) -> Option<CstObject> {
    key.split('.')
        .try_fold(root.object_value()?, |o, k| o.object_value(k))
}

/// The property that holds the last level of `key`: where an emptied server map is removed.
fn parent_of_last(root: &CstRootNode, key: &str) -> Option<jsonc_parser::cst::CstObjectProp> {
    let (parents, last) = key.rsplit_once('.').unwrap_or(("", key));
    let parent = if parents.is_empty() {
        root.object_value()?
    } else {
        servers(root, parents)?
    };
    parent.get(last)
}

fn to_cst(v: &Value) -> CstInputValue {
    match v {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(a) => CstInputValue::Array(a.iter().map(to_cst).collect()),
        Value::Object(m) => {
            CstInputValue::Object(m.iter().map(|(k, v)| (k.clone(), to_cst(v))).collect())
        }
    }
}

/// `v` without a top-level `"type": "stdio"`, the MCP default [`mcp_entry`] spells out: some
/// hosts drop it when they re-save their config, and that is not a change to the entry.
fn without_default_type(v: &Value) -> Value {
    match v {
        Value::Object(m) if m.get("type").and_then(Value::as_str) == Some("stdio") => {
            let mut m = m.clone();
            m.shift_remove("type");
            Value::Object(m)
        }
        _ => v.clone(),
    }
}

/// Write `body` at `path`, gated by `apply` and `report`: a dry run and a [`NO_CHANGES`] report
/// write nothing, and the previous file is backed up first when asked.
///
/// Writes atomically through [`write_atomic`]. When `path` is a symlink (dotfile managers
/// replace host config files with one), the temp file lands beside, and the rename lands on,
/// the file it points to, so the symlink itself survives.
pub fn write(apply: &Apply, path: &Path, body: &str, report: &str) -> Result<()> {
    if !apply.writes(report) {
        return Ok(());
    }
    if apply.backup {
        file_backup::backup(path).with_context(|| path.display().to_string())?;
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).ok();
    }
    let target = link_target(path);
    write_atomic(&target, body).with_context(|| target.display().to_string())
}

/// The file `path` resolves to. `canonicalize` fails on a dangling link (a dotfile manager's
/// link whose target is not there yet), and the rename then replaced the link with a plain
/// file; following the links by hand writes the target and keeps the link. A target whose
/// directory is gone fails the write instead of being created.
fn link_target(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    // 40: the kernel's own ELOOP limit, so a cycle ends.
    for _ in 0..40 {
        if let Ok(real) = fs::canonicalize(&p) {
            return real;
        }
        match fs::read_link(&p) {
            Ok(next) => p = p.parent().unwrap_or(Path::new("")).join(next),
            Err(_) => break,
        }
    }
    p
}

/// [`write`]'s atomic swap: write `body` to a sibling temp file, copy `target`'s permissions
/// onto it when `target` exists (a 0600 `~/.claude.json` must stay 0600 on Unix), then
/// `fs::rename` the temp file over `target` — atomic on one filesystem. Any failed step cleans
/// up the temp file before returning the error.
///
/// On Windows, `MoveFileExW(REPLACE_EXISTING)` fails when the destination has the read-only
/// attribute. Copying that bit onto the temp file (as Unix mode copy would) then made every
/// update of a read-only host config fail; clear it on the destination instead. Temp names
/// include a nanos suffix so two writes in the same process cannot share one leftover file.
pub fn write_atomic(target: &Path, body: &str) -> Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(
        ".{name}.agent-host-config-tmp-{}-{nanos}",
        std::process::id()
    ));
    let result: Result<()> = (|| -> Result<()> {
        fs::write(&tmp, body)?;
        #[cfg(unix)]
        if let Ok(meta) = fs::metadata(target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        #[cfg(windows)]
        clear_readonly(target);
        fs::rename(&tmp, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// `MOVEFILE_REPLACE_EXISTING` cannot replace a read-only file; drop that bit.
#[cfg(windows)]
// Windows-only, so the world-writable risk the lint names for Unix cannot arise here.
#[allow(clippy::permissions_set_readonly_false)]
fn clear_readonly(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    let mut perms = meta.permissions();
    if !perms.readonly() {
        return;
    }
    perms.set_readonly(false);
    let _ = fs::set_permissions(path, perms);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const APPLY: Apply = Apply {
        dry_run: false,
        backup: false,
    };

    fn tmp() -> TempDir {
        TempDir::new().unwrap()
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn register(path: &Path) -> String {
        register_mcp(&APPLY, path, "demo", "demo-bin", &["mcp"]).unwrap()
    }

    /// A host config as a hand-edited file looks: four-space indent, comments, a trailing comma,
    /// keys in no alphabetical order, escaped and raw non-ASCII text, numbers in several
    /// spellings, a foreign server, and keys on both sides of `mcpServers`.
    const FOREIGN: &str = r#"{
    // my editor settings
    "zeta": {
        "theme": "dark",
        "nested": [1, 2.50, 1e3, 12345678901234567890123, true, null],
    },
    "mcpServers": {
        "other": {
            "type": "stdio",
            "command": "npx",
            "args": ["-y", "café"]
        }
    },
    "alpha": "Ünïcode ✓", /* inline */
    "numStartups": 42,
}
"#;

    /// T1.18 done criterion: registering and unregistering our entry leaves every foreign key
    /// byte-for-byte as the host wrote it.
    #[test]
    fn round_trip_keeps_foreign_keys_byte_for_byte() {
        let dir = tmp();
        let path = dir.path().join(".claude.json");
        fs::write(&path, FOREIGN).unwrap();

        assert_eq!(register(&path), "mcpServers.demo: demo-bin mcp");
        let registered = read(&path);
        for kept in [
            "// my editor settings",
            "[1, 2.50, 1e3, 12345678901234567890123, true, null],",
            r#""args": ["-y", "café"]"#,
            r#""alpha": "Ünïcode ✓", /* inline */"#,
            r#""numStartups": 42,"#,
        ] {
            assert!(registered.contains(kept), "{kept} lost:\n{registered}");
        }
        assert_eq!(
            entry_at(&path, "mcpServers", "demo").unwrap(),
            Some(mcp_entry("demo-bin", &["mcp"]))
        );

        assert_eq!(
            unregister_mcp(&APPLY, &path, "demo").unwrap(),
            "- mcpServers.demo"
        );
        assert_eq!(read(&path), FOREIGN);
    }

    /// Strict-JSON, two-space, exotic numbers and no trailing newline: the shape a JS host
    /// writes. Registering a server where none exists keeps every other byte, and unregistering
    /// takes the emptied map with it.
    #[test]
    fn round_trip_without_a_server_map_restores_the_file() {
        let dir = tmp();
        let path = dir.path().join("settings.json");
        let raw = "{\n  \"b\": 1e+21,\n  \"a\": {\n    \"z\": 0.10,\n    \"y\": \"\\/\"\n  }\n}";
        fs::write(&path, raw).unwrap();

        register(&path);
        let registered = read(&path);
        assert!(registered.contains("\"mcpServers\""), "{registered}");
        assert!(registered.contains("\"b\": 1e+21"), "{registered}");
        assert!(registered.contains("\"z\": 0.10"), "{registered}");
        unregister_mcp(&APPLY, &path, "demo").unwrap();
        assert_eq!(read(&path), raw);
    }

    #[test]
    fn mcp_registration_is_idempotent_and_leaves_foreign_servers() {
        let dir = tmp();
        let path = dir.path().join("mcp.json");
        fs::write(&path, r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();
        assert_eq!(register(&path), "mcpServers.demo: demo-bin mcp");
        let once = read(&path);
        assert_eq!(register(&path), NO_CHANGES);
        assert_eq!(read(&path), once);
        assert_eq!(
            unregister_mcp(&APPLY, &path, "demo").unwrap(),
            "- mcpServers.demo"
        );
        assert_eq!(unregister_mcp(&APPLY, &path, "demo").unwrap(), NO_CHANGES);
        let raw = read(&path);
        assert!(raw.contains("other"), "{raw}");
        assert!(!raw.contains("demo"), "{raw}");
    }

    /// A host that re-saves its config drops the default `"type": "stdio"`; that is not a change.
    #[test]
    fn a_missing_default_type_counts_as_the_same_entry() {
        let dir = tmp();
        let path = dir.path().join("mcp.json");
        let raw = r#"{"mcpServers": {"demo": {"command": "demo-bin", "args": ["mcp"]}}}"#;
        fs::write(&path, raw).unwrap();
        assert_eq!(register(&path), NO_CHANGES);
        assert_eq!(read(&path), raw);
    }

    #[test]
    fn register_replaces_a_stale_entry_in_place() {
        let dir = tmp();
        let path = dir.path().join("mcp.json");
        let raw = r#"{"mcpServers":{"demo":{"command":"old"},"other":{"command":"x"}}}"#;
        fs::write(&path, raw).unwrap();
        register(&path);
        let after = read(&path);
        assert!(after.find("\"demo\"") < after.find("\"other\""), "{after}");
        assert!(
            after.contains("\"demo-bin\"") && !after.contains("\"old\""),
            "{after}"
        );
        assert!(after.contains(r#""other":{"command":"x"}"#), "{after}");
    }

    #[test]
    fn nested_and_alternative_keys_work_and_only_the_last_level_is_dropped() {
        let dir = tmp();
        let path = dir.path().join("zcode.json");
        fs::write(&path, r#"{"mcp":{"servers":{},"keep":1}}"#).unwrap();
        let entry = json!({"type": "local", "command": ["demo-bin", "mcp"]});
        assert_eq!(
            register_server(
                &APPLY,
                &path,
                "mcp.servers",
                "demo",
                entry.clone(),
                "demo-bin mcp"
            )
            .unwrap(),
            "mcp.servers.demo: demo-bin mcp"
        );
        assert_eq!(entry_at(&path, "mcp.servers", "demo").unwrap(), Some(entry));
        assert_eq!(entry_at(&path, "mcp.servers", "nope").unwrap(), None);
        assert_eq!(entry_at(&path, "nope.servers", "demo").unwrap(), None);
        unregister_server(&APPLY, &path, "mcp.servers", "demo").unwrap();
        let after: Value = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(
            after,
            json!({"mcp": {"keep": 1}}),
            "the emptied last level goes, its parent stays"
        );
    }

    #[test]
    fn a_missing_or_blank_file_is_an_empty_document() {
        let dir = tmp();
        let path = dir.path().join("new/dir/mcp.json");
        assert_eq!(entry_at(&path, "mcpServers", "demo").unwrap(), None);
        assert_eq!(unregister_mcp(&APPLY, &path, "demo").unwrap(), NO_CHANGES);
        assert!(!path.exists(), "nothing to remove must not create the file");
        register(&path);
        assert!(path.exists(), "missing parent directories are created");
        let body = read(&path);
        assert!(
            body.ends_with("}\n"),
            "a new file ends with a newline: {body:?}"
        );
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            parsed["mcpServers"]["demo"],
            mcp_entry("demo-bin", &["mcp"])
        );

        fs::write(&path, "  \n").unwrap();
        assert_eq!(register(&path), "mcpServers.demo: demo-bin mcp");
        assert_eq!(
            read(&path),
            body,
            "a blank file is the same as a missing one"
        );
    }

    /// A file that is not JSON is refused with the file and the position named, and is never
    /// rewritten.
    #[test]
    fn a_file_that_is_not_json_is_refused_and_left_alone() {
        let dir = tmp();
        let path = dir.path().join("settings.json");
        let raw = "{\n  \"theme\": x\n}\n";
        fs::write(&path, raw).unwrap();
        let err = register_mcp(&APPLY, &path, "demo", "demo-bin", &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("settings.json:"), "{err}");
        assert!(err.contains("line 2"), "{err}");
        assert_eq!(read(&path), raw);
        assert!(unregister_mcp(&APPLY, &path, "demo").is_err());
        assert!(entry_at(&path, "mcpServers", "demo").is_err());
    }

    /// A root or server map of another shape is the user's data: an error, never replaced.
    #[test]
    fn a_wrong_shaped_root_or_map_is_an_error_and_left_alone() {
        let dir = tmp();
        let path = dir.path().join("settings.json");
        for raw in [
            r#"[1]"#,
            r#"{"mcpServers": "nope"}"#,
            r#"{"mcpServers": null}"#,
        ] {
            fs::write(&path, raw).unwrap();
            let err = register_mcp(&APPLY, &path, "demo", "demo-bin", &[])
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("settings.json") && err.contains("not an object"),
                "{err}"
            );
            assert_eq!(read(&path), raw);
            assert_eq!(unregister_mcp(&APPLY, &path, "demo").unwrap(), NO_CHANGES);
        }
    }

    #[test]
    fn a_dry_run_edit_reports_and_writes_nothing() {
        let dir = tmp();
        let path = dir.path().join("mcp.json");
        let dry = Apply {
            dry_run: true,
            backup: true,
        };
        assert_eq!(
            register_mcp(&dry, &path, "demo", "demo-bin", &["mcp"]).unwrap(),
            "mcpServers.demo: demo-bin mcp"
        );
        assert!(!path.exists());
        register(&path);
        let before = read(&path);
        assert_eq!(
            unregister_mcp(&dry, &path, "demo").unwrap(),
            "- mcpServers.demo"
        );
        assert_eq!(read(&path), before);
    }

    #[test]
    fn backup_copies_the_old_file_beside_it_before_the_edit() {
        let dir = tmp();
        let path = dir.path().join("mcp.json");
        fs::write(&path, FOREIGN).unwrap();
        let backing = Apply {
            backup: true,
            ..APPLY
        };
        register_mcp(&backing, &path, "demo", "demo-bin", &["mcp"]).unwrap();
        register_mcp(&backing, &path, "demo", "demo-bin", &["mcp"]).unwrap();
        let baks: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .collect();
        assert_eq!(baks.len(), 1, "one backup, none for a no-change run");
        assert_eq!(read(&baks[0].path()), FOREIGN);
    }

    #[test]
    fn dry_run_writes_nothing_and_backup_keeps_the_old_file() {
        let dir = tmp();
        let path = dir.path().join("settings.json");
        let dry = Apply {
            dry_run: true,
            backup: true,
        };
        write(&dry, &path, "{}", "+ something").unwrap();
        assert!(!path.exists(), "a dry run must not create the file");

        write(&APPLY, &path, "one\n", "+ something").unwrap();
        write(&APPLY, &path, "ignored\n", NO_CHANGES).unwrap();
        assert_eq!(read(&path), "one\n");

        let backing = Apply {
            backup: true,
            ..APPLY
        };
        write(&backing, &path, "two\n", "+ something else").unwrap();
        assert_eq!(read(&path), "two\n");
        let baks: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .collect();
        assert_eq!(baks.len(), 1, "one backup per write");
        assert_eq!(read(&baks[0].path()), "one\n");
    }

    #[test]
    fn write_leaves_no_temp_file_behind() {
        let dir = tmp();
        let path = dir.path().join("settings.json");
        write(&APPLY, &path, "one\n", "+ something").unwrap();
        assert_eq!(read(&path), "one\n");
        write(&APPLY, &path, "two\n", "+ something else").unwrap();
        assert_eq!(read(&path), "two\n");
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("-tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
    }

    /// Windows refuses `rename` over a read-only destination. Setup must still update a host
    /// config the user (or another tool) marked read-only.
    #[cfg(windows)]
    #[test]
    fn write_replaces_a_readonly_file() {
        let dir = tmp();
        let path = dir.path().join("settings.json");
        fs::write(&path, "old\n").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms).unwrap();
        assert!(fs::metadata(&path).unwrap().permissions().readonly());
        write(&APPLY, &path, "new\n", "+ something").unwrap();
        assert_eq!(read(&path), "new\n");
    }

    #[cfg(unix)]
    #[test]
    fn write_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tmp();
        let path = dir.path().join(".claude.json");
        fs::write(&path, "{}\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        write(&APPLY, &path, "{\"a\":1}\n", "+ something").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "write must not loosen an existing file's permissions"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_through_a_symlink_updates_the_target_and_keeps_the_link() {
        let dir = tmp();
        let real = dir.path().join("real.json");
        fs::write(&real, "{}\n").unwrap();
        let link = dir.path().join("linked.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write(&APPLY, &link, "{\"a\":1}\n", "+ something").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "write must not replace the symlink with a plain file"
        );
        assert_eq!(read(&real), "{\"a\":1}\n");
        assert_eq!(read(&link), "{\"a\":1}\n");
    }

    /// A relative link to a file not created yet was replaced by a plain file.
    #[cfg(unix)]
    #[test]
    fn write_through_a_dangling_symlink_creates_the_target_and_keeps_the_link() {
        let dir = tmp();
        fs::create_dir_all(dir.path().join("dots")).unwrap();
        let link = dir.path().join("linked.json");
        std::os::unix::fs::symlink(Path::new("dots/real.json"), &link).unwrap();

        write(&APPLY, &link, "{}\n", "+ something").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(read(&dir.path().join("dots/real.json")), "{}\n");
    }
}
