//! Schema-checked TOML config helpers, generic over the caller's config type.
//!
//! Three jobs every app with a config file wrote for itself:
//!
//! - [`schema`]: the JSON Schema of the config types, and a check that the
//!   committed copy is current.
//! - [`layers`] and [`env`]: layered loading (`figment`) with named layers,
//!   environment variables that may contain `_`, and a provenance table that
//!   says which layer, file or variable supplied each key.
//! - [`edit`]: changing one key of a TOML file in place, comments and layout
//!   kept, and refusing a result the types would reject.
//!
//! The crate is the one place that imports `figment`, `toml_edit` and
//! `schemars` for config work; a caller keeps its own types and layer order.

pub mod edit;
pub mod env;
mod error;
pub mod layers;
mod reveal;
pub mod schema;

pub use edit::{
    Edit, TomlValue, parse_value, plan_edit, plan_edit_foreign, read_entry, value_from_json,
};
pub use env::EnvLayer;
pub use error::{Error, Result};
pub use layers::{Layers, Leaf, Loaded, Origin, Provenance, find_up, git_root};
pub use schema::{check_schema, schema_text};
