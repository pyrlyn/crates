//! One error type for every helper, so a caller maps it once. Every variant
//! that comes from a file names that file.

use std::path::PathBuf;

use crate::layers::Origin;

/// Why a config helper failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Reading or writing `path` failed.
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `path` is not valid TOML.
    #[error("{}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
    /// `path` is valid TOML but does not fit the config types.
    #[error("{}: {message}", path.display())]
    Invalid { path: PathBuf, message: String },
    /// A layered load failed; `origin` is the layer that supplied the
    /// offending value, when the layering can tell.
    #[error("invalid config{}: {message}", origin_suffix(origin))]
    Layered {
        origin: Option<Origin>,
        message: String,
    },
    /// A dotted key with no name in it (`""`, `a..b`).
    #[error("empty key")]
    EmptyKey,
    /// A dotted key walks through a value that is not a table.
    #[error("`{part}` in `{key}` is not a table")]
    NotATable { part: String, key: String },
    /// A JSON value with no TOML form a config key takes (`null`).
    #[error("`{key}` cannot be set to {value}")]
    UnsupportedValue { key: String, value: String },
    /// The committed JSON Schema is not what the types generate.
    #[error("{} is stale ({detail}); regenerate it with {bless_var}=1 and commit it", path.display())]
    StaleSchema {
        path: PathBuf,
        detail: String,
        bless_var: String,
    },
    /// The schema or a value could not be rendered as JSON.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn origin_suffix(origin: &Option<Origin>) -> String {
    origin
        .as_ref()
        .map_or_else(String::new, |o| format!(" in {o}"))
}

/// `Result` with this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
