// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! The one owner of this crate's config I/O: the two embedded TOML files
//! (`models.toml`, `prices.toml`), the parser every layer shares, and the
//! JSON Schemas generated from the types they deserialize into. Nothing else
//! in the crate touches `figment`, so the file shape and its error text live
//! in one place.

use figment::Figment;
use figment::providers::{Format, Toml};
use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;

use llm_wire::ProviderModel;

use crate::catalog::ModelsFile;
use crate::price::PriceFile;

pub(crate) const MODELS_TOML: &str = include_str!("../models.toml");
pub(crate) const PRICES_TOML: &str = include_str!("../prices.toml");

/// Parses TOML text into `T`. The error is the parser's message, which the
/// caller wraps in its own error type.
pub(crate) fn parse<T: DeserializeOwned>(content: &str) -> Result<T, String> {
    Figment::from(Toml::string(content))
        .extract()
        .map_err(|e| e.to_string())
}

/// The embedded built-in model rows.
pub(crate) fn builtin_models() -> Result<Vec<ProviderModel>, String> {
    Ok(parse::<ModelsFile>(MODELS_TOML)?.model)
}

fn schema_json<T: JsonSchema>() -> String {
    // A schema is plain data, so serializing it cannot fail; the empty
    // fallback only exists to keep this path free of `expect`.
    serde_json::to_string_pretty(&schema_for!(T)).unwrap_or_default()
}

/// JSON Schema (pretty-printed) of a `models.toml`-shaped file.
pub fn models_schema() -> String {
    schema_json::<ModelsFile>()
}

/// JSON Schema (pretty-printed) of a `prices.toml`-shaped file, which is
/// also the shape of a user price file.
pub fn prices_schema() -> String {
    schema_json::<PriceFile>()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Compares a committed schema with the one the types generate. Run
    /// with `UPDATE_SCHEMAS=1` to rewrite the file after an intended change.
    fn assert_schema_fresh(file: &str, generated: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("schema")
            .join(file);
        let want = format!("{generated}\n");
        if std::env::var_os("UPDATE_SCHEMAS").is_some() {
            std::fs::write(&path, &want).expect("write schema");
            return;
        }
        let have = std::fs::read_to_string(&path).expect("read committed schema");
        assert!(
            have == want,
            "schema/{file} is stale; run `UPDATE_SCHEMAS=1 cargo test -p llm-catalog` and commit it"
        );
    }

    #[test]
    fn committed_models_schema_matches_the_types() {
        assert_schema_fresh("models.schema.json", &models_schema());
    }

    #[test]
    fn committed_prices_schema_matches_the_types() {
        assert_schema_fresh("prices.schema.json", &prices_schema());
    }

    #[test]
    fn embedded_models_parse_and_have_unique_ids() {
        let models = builtin_models().expect("models.toml parses");
        assert_eq!(models.len(), 24);
        let mut ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), models.len(), "duplicate id in models.toml");
    }

    #[test]
    fn a_stale_models_file_with_an_unknown_key_is_rejected() {
        let stale = "[[model]]\nid = \"x\"\ncontext_window = 1\nmax_tokens = 2\n";
        assert!(parse::<ModelsFile>(stale).is_err());
    }
}
