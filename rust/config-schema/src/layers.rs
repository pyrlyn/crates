//! Layered loading on top of `figment`, where every layer has a name and every
//! effective key can say which layer, file or variable supplied it.
//!
//! The caller picks the layers and their order (lowest first); this module
//! adds the three things every app wrote again: a stable layer name in place of
//! figment's own (`"TOML file"`), a per-key provenance table, and an error that
//! names the layer that supplied a bad value.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use figment::providers::{Format, Serialized, Toml};
use figment::value::{Dict, Map, Value};
use figment::{Figment, Metadata, Profile, Provider, Source};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::env::EnvLayer;
use crate::error::{Error, Result};

/// The layer, and the file or variable inside it, that supplied a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// The name the caller gave the layer (`default`, `user`, `env`, ...).
    pub layer: String,
    /// The file path or environment variable name, when the layer has one.
    pub source: Option<String>,
}

impl Origin {
    fn from_metadata(metadata: &Metadata) -> Self {
        let source = match &metadata.source {
            Some(Source::File(path)) => Some(path.display().to_string()),
            Some(Source::Custom(text)) => Some(text.clone()),
            _ => None,
        };
        Self {
            layer: metadata.name.to_string(),
            source,
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Some(source) => write!(f, "{} ({source})", self.layer),
            None => f.write_str(&self.layer),
        }
    }
}

/// One effective leaf of the merged config.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    /// Dotted key, `proxy.port`.
    pub key: String,
    /// The value; `null` for an unset `Option`.
    pub value: serde_json::Value,
    /// Where it came from.
    pub origin: Origin,
}

/// Origin of every leaf, keyed by dotted path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provenance(BTreeMap<String, Origin>);

impl Provenance {
    /// The origin of `key`, if it is a leaf.
    pub fn get(&self, key: &str) -> Option<&Origin> {
        self.0.get(key)
    }

    /// Every `(key, origin)`, sorted by key.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Origin)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// A typed config and where each of its keys came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded<T> {
    pub config: T,
    pub provenance: Provenance,
}

/// Renames a provider's layer, and optionally pins what it reports as its
/// source, so figment's own labels never reach the user.
struct Named<P> {
    inner: P,
    name: Cow<'static, str>,
    source: Option<String>,
}

impl<P: Provider> Provider for Named<P> {
    fn metadata(&self) -> Metadata {
        let mut metadata = self.inner.metadata();
        metadata.name = self.name.clone();
        if let Some(source) = &self.source {
            metadata.source = Some(Source::Custom(source.clone()));
        }
        metadata
    }

    fn data(&self) -> std::result::Result<Map<Profile, Dict>, figment::Error> {
        self.inner.data()
    }

    fn profile(&self) -> Option<Profile> {
        self.inner.profile()
    }
}

/// An ordered stack of named config layers, lowest first. Each call merges one
/// layer over the ones before it.
pub struct Layers {
    figment: Figment,
}

impl Default for Layers {
    fn default() -> Self {
        Self::new()
    }
}

impl Layers {
    /// No layers yet.
    pub fn new() -> Self {
        Self {
            figment: Figment::new(),
        }
    }

    /// A layer holding every key of `value`, usually `T::default()`.
    #[must_use]
    pub fn defaults<T: Serialize>(self, name: &'static str, value: &T) -> Self {
        self.merge(Named {
            inner: Serialized::defaults(value),
            name: name.into(),
            source: None,
        })
    }

    /// A layer parsed from TOML text, such as an embedded `default.toml`.
    #[must_use]
    pub fn toml_text(self, name: &'static str, text: &str) -> Self {
        self.merge(Named {
            inner: Toml::string(text),
            name: name.into(),
            source: None,
        })
    }

    /// A layer read from the TOML file at `path`. A file that does not exist
    /// adds no layer; one that exists and cannot be parsed fails the load,
    /// naming the file. The path is read as given, never searched for in
    /// parent directories (figment's relative-path behaviour).
    #[must_use]
    pub fn file(self, name: &'static str, path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        if !path.exists() {
            return self;
        }
        self.merge(Named {
            inner: Toml::file_exact(path),
            name: name.into(),
            source: None,
        })
    }

    /// A layer from environment variables resolved by `env`. Each variable is
    /// its own provider so the provenance names the variable.
    #[must_use]
    pub fn env(mut self, name: &'static str, env: &EnvLayer) -> Self {
        for var in env.resolve() {
            let mut dict = Dict::new();
            crate::env::insert_dotted(&mut dict, &var.key, var.value);
            self = self.merge(Named {
                inner: DictLayer(dict),
                name: name.into(),
                source: Some(var.name),
            });
        }
        self
    }

    /// The highest layer, for command-line flags: a sparse tree of only the
    /// keys the user set. `null` entries (an unset `Option` flag) are dropped,
    /// because merged in they would blank the value below.
    pub fn overrides<S: Serialize>(self, name: &'static str, tree: &S) -> Result<Self> {
        let mut json = serde_json::to_value(tree)?;
        prune_nulls(&mut json);
        Ok(self.merge(Named {
            inner: Serialized::defaults(json),
            name: name.into(),
            source: None,
        }))
    }

    /// Any other provider, under `name`.
    #[must_use]
    pub fn provider(self, name: &'static str, provider: impl Provider) -> Self {
        self.merge(Named {
            inner: provider,
            name: name.into(),
            source: None,
        })
    }

    fn merge(mut self, provider: impl Provider) -> Self {
        self.figment = self.figment.merge(provider);
        self
    }

    /// The merged config as `T`. A bad value fails with the layer that
    /// supplied it.
    pub fn extract<T: DeserializeOwned>(&self) -> Result<T> {
        self.figment.extract().map_err(layered_error)
    }

    /// The layer that last set `key` (dotted).
    pub fn origin_of(&self, key: &str) -> Option<Origin> {
        self.figment.find_metadata(key).map(Origin::from_metadata)
    }

    /// Every effective leaf with its origin, sorted by key.
    pub fn leaves(&self) -> Result<Vec<Leaf>> {
        let root: Value = self.figment.extract().map_err(layered_error)?;
        let mut out = Vec::new();
        self.walk(&root, &mut Vec::new(), &mut out);
        Ok(out)
    }

    fn walk(&self, value: &Value, path: &mut Vec<String>, out: &mut Vec<Leaf>) {
        if let Value::Dict(_, dict) = value {
            for (key, child) in dict {
                path.push(key.clone());
                self.walk(child, path, out);
                path.pop();
            }
            return;
        }
        let Some(metadata) = self.figment.get_metadata(value.tag()) else {
            return;
        };
        out.push(Leaf {
            key: path.join("."),
            value: serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
            origin: Origin::from_metadata(metadata),
        });
    }

    /// The typed config and the origin of each of its keys.
    pub fn load<T: DeserializeOwned>(&self) -> Result<Loaded<T>> {
        let config = self.extract()?;
        let provenance = Provenance(
            self.leaves()?
                .into_iter()
                .map(|leaf| (leaf.key, leaf.origin))
                .collect(),
        );
        Ok(Loaded { config, provenance })
    }
}

/// A fixed dictionary as a provider.
struct DictLayer(Dict);

impl Provider for DictLayer {
    fn metadata(&self) -> Metadata {
        Metadata::default()
    }

    fn data(&self) -> std::result::Result<Map<Profile, Dict>, figment::Error> {
        Ok(Profile::Default.collect(self.0.clone()))
    }
}

pub(crate) fn layered_error(error: figment::Error) -> Error {
    let origin = error.metadata.as_ref().map(Origin::from_metadata);
    let message = if error.path.is_empty() {
        error.kind.to_string()
    } else {
        format!("{} for key `{}`", error.kind, error.path.join("."))
    };
    Error::Layered { origin, message }
}

fn prune_nulls(value: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = value {
        map.retain(|_, v| !v.is_null());
        map.values_mut().for_each(prune_nulls);
    }
}

/// The nearest directory at or above `start` that holds an entry called
/// `name`, found without a subprocess. `None` when no ancestor has one.
pub fn find_up(start: &Path, name: &str) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(name).exists())
        .map(Path::to_path_buf)
}

/// The root of the git checkout holding `start`: the nearest ancestor with a
/// `.git` entry (a directory in a clone, a file in a worktree). `None` outside
/// a repository.
pub fn git_root(start: &Path) -> Option<PathBuf> {
    find_up(start, ".git")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Config {
        port: u16,
        host: String,
        tags: Vec<String>,
        note: Option<String>,
        proxy: Proxy,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Proxy {
        openai_upstream: String,
        dry_run: bool,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                port: 8790,
                host: "localhost".into(),
                tags: Vec::new(),
                note: None,
                proxy: Proxy {
                    openai_upstream: "https://api.openai.com".into(),
                    dry_run: false,
                },
            }
        }
    }

    fn base() -> Layers {
        Layers::new().defaults("default", &Config::default())
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_higher_layer_wins_and_is_named_as_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let user = write(dir.path(), "user.toml", "port = 1\nhost = \"user\"\n");
        let project = write(dir.path(), "project.toml", "port = 2\n");
        let loaded = base()
            .file("user", &user)
            .file("project", &project)
            .load::<Config>()
            .unwrap();
        assert_eq!(loaded.config.port, 2);
        assert_eq!(loaded.config.host, "user");
        let prov = &loaded.provenance;
        assert_eq!(prov.get("port").unwrap().layer, "project");
        assert_eq!(prov.get("host").unwrap().layer, "user");
        assert_eq!(prov.get("proxy.dry_run").unwrap().layer, "default");
        assert_eq!(
            prov.get("port").unwrap().source.as_deref(),
            Some(project.to_str().unwrap())
        );
    }

    #[test]
    fn a_missing_file_adds_no_layer_and_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = base()
            .file("user", dir.path().join("absent.toml"))
            .load::<Config>()
            .unwrap();
        assert_eq!(loaded.config, Config::default());
    }

    // why: Jail's closure must return figment's large error type.
    #[allow(clippy::result_large_err)]
    #[test]
    fn a_relative_missing_file_is_not_searched_for_in_parent_directories() {
        // figment's own `Toml::file` would walk up and find this one.
        figment::Jail::expect_with(|jail| {
            jail.create_file("up.toml", "port = 9\n")?;
            std::fs::create_dir("below").map_err(|e| e.to_string())?;
            jail.change_dir("below")?;
            let loaded = base().file("user", "up.toml").extract::<Config>();
            assert_eq!(loaded.map_err(|e| e.to_string())?.port, 8790);
            Ok(())
        });
    }

    #[test]
    fn an_invalid_file_names_the_file_and_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let user = write(dir.path(), "user.toml", "port = \"eighty\"\n");
        let err = base().file("user", &user).load::<Config>().unwrap_err();
        let message = err.to_string();
        assert!(message.contains("user"), "{message}");
        assert!(message.contains("user.toml"), "{message}");
        assert!(message.contains("`port`"), "{message}");
    }

    #[test]
    fn an_unknown_key_in_a_layer_is_rejected_by_the_types() {
        let layers = base().toml_text("embedded", "prot = 1\n");
        let err = layers.extract::<Config>().unwrap_err();
        assert!(err.to_string().contains("prot"), "{err}");
    }

    #[test]
    fn an_unparsable_file_fails_the_load_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let user = write(dir.path(), "user.toml", "port = \n");
        let err = base().file("user", &user).extract::<Config>().unwrap_err();
        assert!(err.to_string().contains("user.toml"), "{err}");
    }

    #[test]
    fn overrides_skip_unset_options_and_win_over_everything() {
        #[derive(Serialize)]
        struct Flags {
            port: Option<u16>,
            host: Option<String>,
        }
        let dir = tempfile::tempdir().unwrap();
        let user = write(dir.path(), "user.toml", "port = 1\nhost = \"user\"\n");
        let flags = Flags {
            port: None,
            host: Some("flag".into()),
        };
        let loaded = base()
            .file("user", &user)
            .overrides("flag", &flags)
            .unwrap()
            .load::<Config>()
            .unwrap();
        assert_eq!(
            loaded.config.port, 1,
            "an unset flag must not blank the key"
        );
        assert_eq!(loaded.config.host, "flag");
        assert_eq!(loaded.provenance.get("host").unwrap().layer, "flag");
        assert_eq!(loaded.provenance.get("port").unwrap().layer, "user");
    }

    #[test]
    fn leaves_lists_every_key_sorted_with_null_for_an_unset_option() {
        let leaves = base().leaves().unwrap();
        let keys: Vec<&str> = leaves.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "host",
                "note",
                "port",
                "proxy.dry_run",
                "proxy.openai_upstream",
                "tags"
            ]
        );
        let note = leaves.iter().find(|l| l.key == "note").unwrap();
        assert!(note.value.is_null());
        let tags = leaves.iter().find(|l| l.key == "tags").unwrap();
        assert_eq!(tags.value, serde_json::json!([]));
    }

    #[test]
    fn origin_of_answers_for_one_key() {
        let layers = base().toml_text("embedded", "port = 3\n");
        assert_eq!(layers.origin_of("port").unwrap().layer, "embedded");
        assert_eq!(layers.origin_of("host").unwrap().layer, "default");
        assert!(layers.origin_of("absent").is_none());
    }

    #[test]
    fn an_origin_displays_its_layer_and_source() {
        let plain = Origin {
            layer: "default".into(),
            source: None,
        };
        assert_eq!(plain.to_string(), "default");
        let env = Origin {
            layer: "env".into(),
            source: Some("APP_PORT".into()),
        };
        assert_eq!(env.to_string(), "env (APP_PORT)");
    }

    #[test]
    fn git_root_finds_a_dot_git_directory_or_file_and_none_outside() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let deep = root.join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(git_root(&deep), None);
        // A worktree's `.git` is a file.
        std::fs::write(root.join(".git"), "gitdir: elsewhere\n").unwrap();
        assert_eq!(git_root(&deep), Some(root.clone()));
        assert_eq!(find_up(&deep, "missing"), None);
    }
}
