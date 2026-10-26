//! Check `[profile.*]` sections of Cargo manifests against a template.
//!
//! Profiles are resolved the way Cargo resolves them (built-in defaults for `dev` and
//! `release`, `test` inherits `dev`, `bench` inherits `release`, custom profiles follow
//! `inherits`), then every key of every template profile is compared with the effective
//! value. Differences come out as [`Diff`] lines: `ADD` what the manifest must gain,
//! `REMOVE` what it must lose.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

/// The template bundled with the binary.
pub const BUNDLED_TEMPLATE: &str = include_str!("../templates/profile-template.toml");

/// Error that maps to exit code 2.
#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn err(msg: impl Into<String>) -> Error {
    Error(msg.into())
}

/// One profile: its own keys and its `package."*"` keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profile {
    pub keys: BTreeMap<String, Value>,
    pub all_packages: BTreeMap<String, Value>,
}

/// `[profile.*]` of a template or manifest, as written (nothing resolved).
pub type Profiles = BTreeMap<String, Profile>;

/// Read `[profile.*]` out of a parsed TOML document.
pub fn profiles_from_table(doc: &Table) -> Result<Profiles, Error> {
    let mut out = Profiles::new();
    let Some(profile) = doc.get("profile") else {
        return Ok(out);
    };
    let profile = profile
        .as_table()
        .ok_or_else(|| err("`profile` is not a table"))?;
    for (name, body) in profile {
        let body = body
            .as_table()
            .ok_or_else(|| err(format!("`profile.{name}` is not a table")))?;
        let mut p = Profile::default();
        for (k, v) in body {
            if k == "package" {
                if let Some(star) = v.as_table().and_then(|t| t.get("*")) {
                    let star = star.as_table().ok_or_else(|| {
                        err(format!("`profile.{name}.package.\"*\"` is not a table"))
                    })?;
                    for (sk, sv) in star {
                        p.all_packages.insert(sk.clone(), sv.clone());
                    }
                }
                // Named package overrides and build-override are outside the template model.
                continue;
            }
            if k == "build-override" {
                continue;
            }
            p.keys.insert(k.clone(), v.clone());
        }
        out.insert(name.clone(), p);
    }
    Ok(out)
}

/// Parse a template (any TOML with `[profile.*]` sections).
pub fn parse_template(text: &str) -> Result<Profiles, Error> {
    let doc: Table = text.parse().map_err(|e| err(format!("template: {e}")))?;
    let p = profiles_from_table(&doc)?;
    if p.is_empty() {
        return Err(err("template has no [profile.*] sections"));
    }
    Ok(p)
}

fn builtin_defaults(name: &str) -> Option<BTreeMap<String, Value>> {
    let (opt, debug, assertions, incremental, cgu) = match name {
        "dev" => (0, true, true, true, 256),
        "release" => (3, false, false, false, 16),
        _ => return None,
    };
    let mut m = BTreeMap::new();
    m.insert("opt-level".into(), Value::Integer(opt));
    m.insert("debug".into(), Value::Boolean(debug));
    m.insert("strip".into(), Value::String("none".into()));
    m.insert("debug-assertions".into(), Value::Boolean(assertions));
    m.insert("overflow-checks".into(), Value::Boolean(assertions));
    m.insert("lto".into(), Value::Boolean(false));
    m.insert("panic".into(), Value::String("unwind".into()));
    m.insert("incremental".into(), Value::Boolean(incremental));
    m.insert("codegen-units".into(), Value::Integer(cgu));
    m.insert("rpath".into(), Value::Boolean(false));
    Some(m)
}

/// The parent a profile inherits from, if any.
fn parent_of(name: &str, explicit: Option<&Profile>) -> Result<Option<String>, Error> {
    match name {
        "dev" | "release" => Ok(None),
        "test" => Ok(Some("dev".into())),
        "bench" => Ok(Some("release".into())),
        _ => match explicit.and_then(|p| p.keys.get("inherits")) {
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(err(format!("profile.{name}: `inherits` must be a string"))),
            None => Err(err(format!(
                "profile.{name}: custom profile without `inherits`"
            ))),
        },
    }
}

/// Effective profile `name`, or `None` if a custom profile is not defined. Errors on cycles.
pub fn resolve(profiles: &Profiles, name: &str) -> Result<Option<Profile>, Error> {
    let builtin = matches!(name, "dev" | "release" | "test" | "bench");
    if !builtin && !profiles.contains_key(name) {
        return Ok(None);
    }
    // Chain from `name` up to a root.
    let mut chain = vec![name.to_string()];
    let mut seen = BTreeSet::from([name.to_string()]);
    let mut cur = name.to_string();
    while let Some(parent) = parent_of(&cur, profiles.get(&cur))? {
        if !seen.insert(parent.clone()) {
            chain.push(parent);
            return Err(err(format!("inherits cycle: {}", chain.join(" -> "))));
        }
        if !matches!(parent.as_str(), "dev" | "release" | "test" | "bench")
            && !profiles.contains_key(&parent)
        {
            return Err(err(format!(
                "profile.{cur}: inherits unknown profile `{parent}`"
            )));
        }
        chain.push(parent.clone());
        cur = parent;
    }
    let mut eff = Profile {
        keys: builtin_defaults(&cur).unwrap_or_default(),
        all_packages: BTreeMap::new(),
    };
    for p in chain.iter().rev() {
        if let Some(explicit) = profiles.get(p) {
            for (k, v) in &explicit.keys {
                eff.keys.insert(k.clone(), v.clone());
            }
            for (k, v) in &explicit.all_packages {
                eff.all_packages.insert(k.clone(), v.clone());
            }
        }
    }
    // `inherits` is reported as written on the profile itself, not taken from ancestors.
    eff.keys.remove("inherits");
    if let Some(v) = profiles.get(name).and_then(|p| p.keys.get("inherits")) {
        eff.keys.insert("inherits".into(), v.clone());
    }
    Ok(Some(eff))
}

/// Canonical form so equivalent spellings compare equal.
pub fn normalize(key: &str, v: &Value) -> Value {
    use Value::{Boolean as B, Integer as I, String as S};
    match (key, v) {
        ("debug", B(false) | I(0)) => S("none".into()),
        ("debug", S(s)) if s == "none" => S("none".into()),
        ("debug", I(1)) => S("limited".into()),
        ("debug", B(true) | I(2)) => S("full".into()),
        ("lto", B(true)) => S("fat".into()),
        ("lto", S(s)) if s == "fat" => S("fat".into()),
        ("lto", B(false)) => S("false".into()),
        ("strip", B(false)) => S("none".into()),
        ("strip", B(true)) => S("symbols".into()),
        ("opt-level", S(s)) => match s.parse::<i64>() {
            Ok(n) => I(n),
            Err(_) => S(s.clone()),
        },
        _ => v.clone(),
    }
}

/// `ADD` or `REMOVE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Remove,
}

/// One line of output.
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    pub op: Op,
    pub manifest: PathBuf,
    /// `profile.<name>` or `profile.<name>.package."*"`.
    pub section: String,
    pub key: String,
    pub value: Value,
}

impl fmt::Display for Diff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self.op {
            Op::Add => "ADD",
            Op::Remove => "REMOVE",
        };
        write!(
            f,
            "{op} {} {} {} = {}",
            self.manifest.display(),
            self.section,
            self.key,
            self.value
        )
    }
}

fn compare_tables(
    out: &mut Vec<Diff>,
    manifest: &Path,
    section: &str,
    expected: &BTreeMap<String, Value>,
    effective: &BTreeMap<String, Value>,
    explicit: Option<&BTreeMap<String, Value>>,
) {
    let mk = |op, key: &str, value: &Value| Diff {
        op,
        manifest: manifest.to_path_buf(),
        section: section.to_string(),
        key: key.to_string(),
        value: value.clone(),
    };
    let explicit_get = |k: &str| explicit.and_then(|t| t.get(k));
    for (k, want) in expected {
        let ok = effective
            .get(k)
            .is_some_and(|have| normalize(k, have) == normalize(k, want));
        if ok {
            continue;
        }
        if let Some(have) = explicit_get(k) {
            out.push(mk(Op::Remove, k, have));
        }
        out.push(mk(Op::Add, k, want));
    }
    if let Some(explicit) = explicit {
        for (k, have) in explicit {
            if !expected.contains_key(k) {
                out.push(mk(Op::Remove, k, have));
            }
        }
    }
}

/// Compare a manifest's profiles with the template.
pub fn check_profiles(
    manifest: &Path,
    template: &Profiles,
    actual: &Profiles,
) -> Result<Vec<Diff>, Error> {
    let mut out = Vec::new();
    for (name, want) in template {
        let section = format!("profile.{name}");
        let pkg_section = format!("profile.{name}.package.\"*\"");
        let explicit = actual.get(name);
        let eff = resolve(actual, name)
            .map_err(|e| err(format!("{}: {e}", manifest.display())))?
            .unwrap_or_default();
        compare_tables(
            &mut out,
            manifest,
            &section,
            &want.keys,
            &eff.keys,
            explicit.map(|p| &p.keys),
        );
        compare_tables(
            &mut out,
            manifest,
            &pkg_section,
            &want.all_packages,
            &eff.all_packages,
            explicit.map(|p| &p.all_packages),
        );
    }
    Ok(out)
}

/// Result for one manifest.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub manifest: PathBuf,
    pub diffs: Vec<Diff>,
    pub warnings: Vec<String>,
}

fn read_manifest(path: &Path) -> Result<Table, Error> {
    let text = fs::read_to_string(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    text.parse()
        .map_err(|e| err(format!("{}: {e}", path.display())))
}

/// Check one manifest file.
pub fn check_manifest(path: &Path, template: &Profiles) -> Result<Report, Error> {
    let doc = read_manifest(path)?;
    let actual = profiles_from_table(&doc)?;
    Ok(Report {
        manifest: path.to_path_buf(),
        diffs: check_profiles(path, template, &actual)?,
        warnings: Vec::new(),
    })
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], t) || (!t.is_empty() && t[0] != '/' && go(p, &t[1..])),
            (Some('?'), Some(c)) if *c != '/' => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    go(&p, &t)
}

fn string_list(t: &Table, key: &str) -> Vec<String> {
    t.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim_end_matches('/').to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The workspace root manifest that `dir` (a package directory) belongs to, if any.
fn owning_workspace(dir: &Path, doc: &Table) -> Option<PathBuf> {
    if let Some(ws) = doc
        .get("package")
        .and_then(|p| p.get("workspace"))
        .and_then(Value::as_str)
    {
        return Some(dir.join(ws).join("Cargo.toml"));
    }
    let mut cur = dir.parent();
    while let Some(anc) = cur {
        let m = anc.join("Cargo.toml");
        if let Ok(adoc) = read_manifest(&m)
            && let Some(ws) = adoc.get("workspace").and_then(Value::as_table)
        {
            let rel = dir
                .strip_prefix(anc)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let excluded = string_list(ws, "exclude")
                .iter()
                .any(|e| rel == *e || rel.starts_with(&format!("{e}/")));
            let member = string_list(ws, "members")
                .iter()
                .any(|g| glob_match(g, &rel));
            return (member && !excluded).then_some(m);
        }
        cur = anc.parent();
    }
    None
}

fn find_manifests(dir: &Path, exclude: &[PathBuf], out: &mut Vec<PathBuf>) -> Result<(), Error> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| err(format!("{}: {e}", dir.display())))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let path = e.path();
        let ft = e.file_type().map_err(|e| err(e.to_string()))?;
        if ft.is_dir() {
            if matches!(name.as_ref(), "target" | "node_modules") || name.starts_with('.') {
                continue;
            }
            if exclude.iter().any(|x| path.starts_with(x)) {
                continue;
            }
            find_manifests(&path, exclude, out)?;
        } else if name == "Cargo.toml" {
            out.push(path);
        }
    }
    Ok(())
}

/// What to do with a manifest.
enum Role {
    /// Workspace root or standalone package: profiles apply.
    Check,
    /// Workspace member: profiles are ignored by Cargo.
    Member(PathBuf),
}

fn role(path: &Path, doc: &Table) -> Role {
    if doc.contains_key("workspace") || !doc.contains_key("package") {
        return Role::Check;
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    match owning_workspace(dir, doc) {
        Some(root) => Role::Member(root),
        None => Role::Check,
    }
}

fn member_warning(doc: &Table, root: &Path) -> Option<String> {
    doc.contains_key("profile").then(|| {
        format!(
            "member crate defines [profile], which Cargo ignores; profiles come from {}",
            root.display()
        )
    })
}

/// Walk `dir` and check every workspace root and standalone package; warn about members
/// that define `[profile]`. Directories in `exclude` (paths under `dir`) are skipped.
pub fn check_workspace(
    dir: &Path,
    exclude: &[PathBuf],
    template: &Profiles,
) -> Result<Vec<Report>, Error> {
    let exclude: Vec<PathBuf> = exclude.iter().map(|x| dir.join(x)).collect();
    let mut manifests = Vec::new();
    find_manifests(dir, &exclude, &mut manifests)?;
    let mut reports = Vec::new();
    for m in manifests {
        let doc = read_manifest(&m)?;
        match role(&m, &doc) {
            Role::Check => reports.push(check_manifest(&m, template)?),
            Role::Member(root) => {
                if let Some(w) = member_warning(&doc, &root) {
                    reports.push(Report {
                        manifest: m,
                        diffs: Vec::new(),
                        warnings: vec![w],
                    });
                }
            }
        }
    }
    Ok(reports)
}

/// Check an explicit path (a `Cargo.toml` or a directory holding one). Members are still
/// checked, with a warning that Cargo ignores their profiles.
pub fn check_path(path: &Path, template: &Profiles) -> Result<Report, Error> {
    let m = if path.is_dir() {
        path.join("Cargo.toml")
    } else {
        path.to_path_buf()
    };
    let doc = read_manifest(&m)?;
    let mut report = check_manifest(&m, template)?;
    if let Role::Member(root) = role(&m, &doc) {
        report.warnings.push(format!(
            "workspace member; Cargo uses the profiles of {} instead",
            root.display()
        ));
        if let Some(w) = member_warning(&doc, &root) {
            report.warnings.push(w);
        }
    }
    Ok(report)
}

/// Plain-text output: `WARN` lines, then `ADD`/`REMOVE` lines.
pub fn render_text(reports: &[Report]) -> String {
    let mut s = String::new();
    for r in reports {
        for w in &r.warnings {
            s.push_str(&format!("WARN {} {w}\n", r.manifest.display()));
        }
        for d in &r.diffs {
            s.push_str(&format!("{d}\n"));
        }
    }
    s
}

fn toml_to_json(v: &Value) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

/// JSON output.
pub fn render_json(reports: &[Report]) -> String {
    let manifests: Vec<_> = reports
        .iter()
        .map(|r| {
            serde_json::json!({
                "manifest": r.manifest.display().to_string(),
                "warnings": r.warnings,
                "diffs": r.diffs.iter().map(|d| serde_json::json!({
                    "op": match d.op { Op::Add => "ADD", Op::Remove => "REMOVE" },
                    "section": d.section,
                    "key": d.key,
                    "value": toml_to_json(&d.value),
                    "toml": d.value.to_string(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let ok = reports.iter().all(|r| r.diffs.is_empty());
    let v = serde_json::json!({ "ok": ok, "manifests": manifests });
    format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Profiles {
        profiles_from_table(&text.parse().unwrap()).unwrap()
    }

    #[test]
    fn bundled_template_parses() {
        let t = parse_template(BUNDLED_TEMPLATE).unwrap();
        assert_eq!(
            t.keys().collect::<Vec<_>>(),
            ["ci", "dev", "dist", "release"]
        );
        assert_eq!(t["dev"].all_packages.len(), 2);
    }

    #[test]
    fn normalization() {
        let n = |k, v| normalize(k, &v);
        assert_eq!(
            n("debug", Value::Boolean(true)),
            n("debug", Value::Integer(2))
        );
        assert_eq!(
            n("debug", Value::Integer(2)),
            n("debug", Value::String("full".into()))
        );
        assert_eq!(
            n("debug", Value::Boolean(false)),
            n("debug", Value::String("none".into()))
        );
        assert_eq!(
            n("debug", Value::Integer(0)),
            n("debug", Value::Boolean(false))
        );
        assert_eq!(
            n("lto", Value::Boolean(true)),
            n("lto", Value::String("fat".into()))
        );
        assert_ne!(
            n("lto", Value::Boolean(false)),
            n("lto", Value::String("off".into()))
        );
        assert_eq!(
            n("opt-level", Value::String("3".into())),
            n("opt-level", Value::Integer(3))
        );
        assert_ne!(
            n("opt-level", Value::String("s".into())),
            n("opt-level", Value::Integer(3))
        );
    }

    #[test]
    fn test_inherits_dev_and_defaults_apply() {
        let a = p("[profile.dev]\nopt-level = 1\n");
        let t = resolve(&a, "test").unwrap().unwrap();
        assert_eq!(t.keys["opt-level"], Value::Integer(1));
        assert_eq!(t.keys["incremental"], Value::Boolean(true));
        let b = resolve(&a, "bench").unwrap().unwrap();
        assert_eq!(b.keys["opt-level"], Value::Integer(3));
    }

    #[test]
    fn custom_chain_and_cycle() {
        let a =
            p("[profile.a]\ninherits = \"b\"\n[profile.b]\ninherits = \"release\"\nlto = true\n");
        let r = resolve(&a, "a").unwrap().unwrap();
        assert_eq!(r.keys["lto"], Value::Boolean(true));
        assert_eq!(r.keys["inherits"], Value::String("b".into()));
        assert!(resolve(&a, "missing").unwrap().is_none());
        let c = p("[profile.a]\ninherits = \"b\"\n[profile.b]\ninherits = \"a\"\n");
        let e = resolve(&c, "a").unwrap_err();
        assert!(e.0.contains("cycle"), "{e}");
    }

    #[test]
    fn diff_lines() {
        let t = p("[profile.dev]\ndebug = \"line-tables-only\"\nopt-level = 0\n");
        let a = p("[profile.dev]\ndebug = 2\npanic = \"abort\"\n");
        let d = check_profiles(Path::new("x/Cargo.toml"), &t, &a).unwrap();
        let lines: Vec<_> = d.iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "REMOVE x/Cargo.toml profile.dev debug = 2",
                "ADD x/Cargo.toml profile.dev debug = \"line-tables-only\"",
                "REMOVE x/Cargo.toml profile.dev panic = \"abort\"",
            ]
        );
    }

    #[test]
    fn globs() {
        assert!(glob_match("rust/*", "rust/a"));
        assert!(!glob_match("rust/*", "rust/a/b"));
        assert!(glob_match("rust/a", "rust/a"));
    }
}
