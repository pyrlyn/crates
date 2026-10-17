//! Environment variables as a config layer.
//!
//! `figment`'s own `Env` splits every `_` into a nesting boundary, which turns
//! `APP_PROXY_OPENAI_UPSTREAM` into `proxy.openai.upstream` and fails a strict
//! parse. This resolver instead matches each name against the key tree of the
//! config's defaults, so field names may contain `_` and a nesting separator
//! is never needed. The process environment is read once, in [`EnvLayer::new`];
//! tests inject variables with [`EnvLayer::with_vars`] and never touch it.

use figment::value::{Dict, Value};
use serde::Serialize;

use crate::error::Result;
use crate::layers::layered_error;

/// One variable resolved to a config key.
pub(crate) struct Resolved {
    /// The variable's own name, for provenance.
    pub(crate) name: String,
    pub(crate) key: String,
    pub(crate) value: Value,
}

/// `PREFIX_*` variables resolved against the keys of a config's defaults.
pub struct EnvLayer {
    prefix: String,
    tree: Dict,
    ignore: Vec<String>,
    vars: Vec<(String, String)>,
}

impl EnvLayer {
    /// Reads the process environment. `prefix` is the variable prefix
    /// including its underscore (`"APP_"`); `defaults` supplies the key tree,
    /// usually `T::default()`.
    pub fn new<T: Serialize>(prefix: &str, defaults: &T) -> Result<Self> {
        let vars = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect();
        Self::with_vars(prefix, defaults, vars)
    }

    /// Like [`EnvLayer::new`] over an explicit list instead of the process
    /// environment, so a test never depends on what the machine has set.
    pub fn with_vars<T: Serialize>(
        prefix: &str,
        defaults: &T,
        vars: Vec<(String, String)>,
    ) -> Result<Self> {
        let tree = match Value::serialize(defaults).map_err(layered_error)? {
            Value::Dict(_, dict) => dict,
            _ => Dict::new(),
        };
        Ok(Self {
            prefix: prefix.to_ascii_uppercase(),
            tree,
            ignore: Vec::new(),
            vars,
        })
    }

    /// Names, without the prefix, that share the prefix but are not config
    /// keys (`APP_HOME`, `APP_CONFIG`), so a strict parse never sees them as
    /// unknown keys. Matched case-insensitively.
    #[must_use]
    pub fn ignore(mut self, names: &[&str]) -> Self {
        self.ignore
            .extend(names.iter().map(|n| n.to_ascii_lowercase()));
        self
    }

    /// Every variable that carries the prefix and is not ignored, sorted by
    /// name so the merge order is the same on every run.
    pub(crate) fn resolve(&self) -> Vec<Resolved> {
        let mut out: Vec<Resolved> = self
            .vars
            .iter()
            .filter_map(|(name, raw)| {
                let upper = name.to_ascii_uppercase();
                let rest = upper.strip_prefix(&self.prefix)?.to_ascii_lowercase();
                if rest.is_empty() || self.ignore.contains(&rest) {
                    return None;
                }
                let parts = key_parts(&self.tree, &rest);
                let value = parse_value(&self.tree, &parts, raw);
                Some(Resolved {
                    name: name.clone(),
                    key: parts.join("."),
                    value,
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

/// Maps a prefix-stripped, lowercased variable name to key parts. Each level
/// takes the longest known name that is the whole rest or a prefix of it
/// followed by `_`; what no known name covers (a user-named table, a typo) is
/// split on `_`, so it still lands somewhere a strict parse can reject.
fn key_parts(tree: &Dict, name: &str) -> Vec<String> {
    let mut rest = name;
    let mut table = Some(tree);
    let mut parts: Vec<String> = Vec::new();
    while let Some(dict) = table {
        let hit = dict
            .iter()
            .filter(|(key, _)| {
                rest.strip_prefix(key.as_str())
                    .is_some_and(|tail| tail.is_empty() || tail.starts_with('_'))
            })
            .max_by_key(|(key, _)| key.len());
        let Some((key, value)) = hit else { break };
        parts.push(key.clone());
        rest = rest[key.len()..].strip_prefix('_').unwrap_or("");
        table = value.as_dict();
    }
    if !rest.is_empty() {
        parts.extend(rest.split('_').map(str::to_string));
    }
    parts
}

/// A key whose default is an array takes a comma-separated list
/// (`A,b` is `["A", "b"]`, an empty value is `[]`, not `[""]`); anything else
/// is parsed the way figment reads a value: bool, number, `[...]`, else text.
fn parse_value(tree: &Dict, parts: &[String], raw: &str) -> Value {
    let mut node = None;
    let mut dict = Some(tree);
    for part in parts {
        node = dict.and_then(|d| d.get(part));
        dict = node.and_then(Value::as_dict);
    }
    let is_array = matches!(node, Some(Value::Array(..)));
    if is_array && !raw.trim_start().starts_with('[') {
        return Value::from(
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(Value::from)
                .collect::<Vec<_>>(),
        );
    }
    raw.parse().unwrap_or_else(|_| Value::from(raw.to_string()))
}

/// Inserts `value` at the dotted path `key`, creating tables on the way and
/// descending into ones that exist, so two keys under one table do not
/// overwrite each other.
pub(crate) fn insert_dotted(dict: &mut Dict, key: &str, value: Value) {
    match key.split_once('.') {
        Some((head, rest)) => {
            let entry = dict
                .entry(head.to_string())
                .or_insert_with(|| Value::from(Dict::new()));
            if let Value::Dict(_, inner) = entry {
                insert_dotted(inner, rest, value);
            } else {
                let mut inner = Dict::new();
                insert_dotted(&mut inner, rest, value);
                *entry = Value::from(inner);
            }
        }
        None => {
            dict.insert(key.to_string(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::Layers;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Config {
        port: u16,
        show_thinking: bool,
        roots: Vec<String>,
        proxy: Proxy,
        tiers: std::collections::BTreeMap<String, Tier>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Proxy {
        port: u16,
        openai_upstream: String,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
    #[serde(default, deny_unknown_fields)]
    struct Tier {
        model: String,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                port: 1,
                show_thinking: false,
                roots: vec!["a".into()],
                proxy: Proxy {
                    port: 2,
                    openai_upstream: "up".into(),
                },
                tiers: [("code".to_string(), Tier { model: "m".into() })].into(),
            }
        }
    }

    fn layer(vars: &[(&str, &str)]) -> EnvLayer {
        let vars = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        EnvLayer::with_vars("APP_", &Config::default(), vars).unwrap()
    }

    fn load(env: &EnvLayer) -> crate::layers::Loaded<Config> {
        Layers::new()
            .defaults("default", &Config::default())
            .env("env", env)
            .load()
            .unwrap()
    }

    #[test]
    fn underscores_in_field_names_are_not_separators() {
        let env = layer(&[
            ("APP_SHOW_THINKING", "true"),
            ("APP_PROXY_OPENAI_UPSTREAM", "https://example.test"),
            ("APP_PROXY_PORT", "9"),
            ("APP_PORT", "7"),
        ]);
        let loaded = load(&env);
        assert!(loaded.config.show_thinking);
        assert_eq!(loaded.config.proxy.openai_upstream, "https://example.test");
        assert_eq!(loaded.config.proxy.port, 9);
        assert_eq!(loaded.config.port, 7, "APP_PORT is port, not proxy.port");
    }

    #[test]
    fn the_variable_name_is_the_provenance() {
        let loaded = load(&layer(&[("app_port", "7")]));
        let origin = loaded.provenance.get("port").unwrap();
        assert_eq!(origin.layer, "env");
        assert_eq!(origin.source.as_deref(), Some("app_port"));
        assert_eq!(
            loaded.provenance.get("proxy.port").unwrap().layer,
            "default"
        );
    }

    #[test]
    fn two_keys_under_one_table_do_not_overwrite_each_other() {
        let env = layer(&[("APP_PROXY_PORT", "9"), ("APP_PROXY_OPENAI_UPSTREAM", "x")]);
        let loaded = load(&env);
        assert_eq!(
            (
                loaded.config.proxy.port,
                loaded.config.proxy.openai_upstream.as_str()
            ),
            (9, "x")
        );
    }

    #[test]
    fn an_array_key_takes_a_comma_list_and_an_empty_value_is_an_empty_list() {
        assert_eq!(
            load(&layer(&[("APP_ROOTS", "x, y ,z")])).config.roots,
            ["x", "y", "z"]
        );
        assert!(load(&layer(&[("APP_ROOTS", "")])).config.roots.is_empty());
        assert_eq!(
            load(&layer(&[("APP_ROOTS", "[\"q\"]")])).config.roots,
            ["q"]
        );
    }

    #[test]
    fn a_name_no_default_covers_is_split_so_a_user_named_table_still_lands() {
        let loaded = load(&layer(&[("APP_TIERS_FAST_MODEL", "small")]));
        assert_eq!(loaded.config.tiers["fast"].model, "small");
        assert_eq!(loaded.config.tiers["code"].model, "m");
    }

    #[test]
    fn ignored_and_foreign_variables_never_reach_the_strict_parse() {
        let env = layer(&[
            ("APP_HOME", "/h"),
            ("OTHER_PORT", "5"),
            ("APP_", "x"),
            ("app_Config", "c"),
        ])
        .ignore(&["home", "CONFIG"]);
        assert_eq!(load(&env).config, Config::default());
    }

    #[test]
    fn a_typo_is_an_unknown_key_error_naming_the_variable_layer() {
        let env = layer(&[("APP_PROT", "1")]);
        let err = Layers::new()
            .defaults("default", &Config::default())
            .env("env", &env)
            .extract::<Config>()
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("prot"), "{message}");
        assert!(message.contains("env (APP_PROT)"), "{message}");
    }

    // why: Jail's closure must return figment's large error type.
    #[allow(clippy::result_large_err)]
    #[test]
    fn the_process_environment_is_read_by_new() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("APP_PORT", "42");
            let env = EnvLayer::new("APP_", &Config::default()).map_err(|e| e.to_string())?;
            assert_eq!(load(&env).config.port, 42);
            Ok(())
        });
    }
}
