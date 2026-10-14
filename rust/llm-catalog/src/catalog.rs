// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! The merged model catalog: one [`ModelRow`] per model id, combining four
//! layers in override order — built-in rows < plugin `[[models]]` rows
//! (fill-only) < the host's configured [`ProviderModel`]s < a user price file.
//! The effort rule (`llm_wire::effort_for`) is a reader of `capabilities`;
//! this module only builds and merges the row.

use std::collections::HashMap;
use std::fmt;

use llm_wire::{Capabilities, Effort, ProviderModel};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config;
use crate::price::{Price, PriceError, PriceTable};

/// The shape of a whole `models.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelsFile {
    pub(crate) model: Vec<ProviderModel>,
}

/// A plugin's `[[models]]` row, as [`Catalog::load`] takes it. The host maps
/// its plugin manifest onto this, so the crate needs no plugin host.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginModel {
    /// Model id as sent on the wire.
    pub id: String,
    /// Context window in tokens.
    pub context_window: Option<u32>,
    /// Max output tokens.
    pub max_output: Option<u32>,
    /// USD per million tokens.
    pub price: Option<PluginPrice>,
}

/// A plugin's `[[models]]` price, USD per million tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginPrice {
    /// Input tokens.
    pub input: f64,
    /// Output tokens.
    pub output: f64,
    /// Cache-read tokens.
    pub cache_read: Option<f64>,
    /// Cache-write tokens.
    pub cache_write: Option<f64>,
}

/// One catalog row: everything the catalog knows about a model id.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRow {
    /// The model id, exactly as sent on the wire (a `ModelId` string).
    pub id: String,
    /// What a person calls the model (`Claude Sonnet 5`), from models.dev's
    /// `name` through `default.toml` (A111) or a config entry. `None` means
    /// a reader shows the id.
    pub display_name: Option<String>,
    /// Context window in tokens, when a layer has supplied one.
    pub context_window: Option<u32>,
    /// Max output tokens, when a layer has supplied one (no source emits
    /// this yet).
    pub max_output: Option<u32>,
    /// Efforts this model supports; empty means "any" — `ProviderModel`'s
    /// own convention, kept so a row built from config matches it exactly.
    pub efforts: Vec<Effort>,
    /// Capability flags, absent where the data doesn't exist yet.
    pub capabilities: Capabilities,
    /// Price, once a `prices.toml` row (built-in or user) or a plugin's
    /// `[[models]]` row prices this id.
    pub price: Option<Price>,
    /// The layer that first created this row (diagnostics). A later
    /// layer that overrides or fills a field does not change it: a built-in
    /// row a plugin filled is still built-in, which is what tells a reader
    /// the plugin could only fill it.
    pub source: RowSource,
}

impl ModelRow {
    fn new(id: String, source: RowSource) -> Self {
        Self {
            id,
            display_name: None,
            context_window: None,
            max_output: None,
            efforts: Vec::new(),
            capabilities: Capabilities::default(),
            price: None,
            source,
        }
    }
}

/// Where a [`ModelRow`] came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowSource {
    /// The embedded `models.toml` or `prices.toml`.
    Builtin,
    /// A granted plugin's `[[models]]` row; holds the plugin id.
    Plugin(String),
    /// The host's configured [`ProviderModel`]s.
    Config,
    /// The user price file.
    UserPrices,
    /// A running local server's report.
    Served,
}

impl fmt::Display for RowSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin => f.write_str("built-in"),
            // Same `plugin:<id>` spelling as the plugin's cost attribution,
            // so one grep finds everything a plugin brought.
            Self::Plugin(id) => write!(f, "plugin:{id}"),
            Self::Config => f.write_str("config"),
            Self::UserPrices => f.write_str("user prices"),
            Self::Served => f.write_str("served"),
        }
    }
}

/// One granted plugin's `[[models]]` rows, as [`Catalog::load`] takes them.
/// Borrowed so this crate stays pure; the caller passes granted plugins only.
#[derive(Debug, Clone, Copy)]
pub struct PluginModels<'a> {
    /// The plugin id: the tie-break between plugins and the `source`.
    pub plugin: &'a str,
    /// Its `[[models]]` rows.
    pub models: &'a [PluginModel],
}

/// A plugin's `[[models]]` price as a catalog price. A missing cache rate
/// falls back to the input rate: billing a cache hit at the full rate can
/// over-count, but never under-counts what the provider charges.
fn plugin_price(plugin: &str, id: &str, decl: &PluginPrice) -> Price {
    Price {
        id: id.to_string(),
        input: decl.input,
        output: decl.output,
        cache_write: decl.cache_write.unwrap_or(decl.input),
        cache_read: decl.cache_read.unwrap_or(decl.input),
        // A plugin row carries no verification date; `source_url` names who
        // supplied the rates instead.
        verified_on: String::new(),
        source_url: format!("plugin:{plugin}"),
    }
}

fn same_rates(a: &Price, b: &Price) -> bool {
    a.input == b.input
        && a.output == b.output
        && a.cache_write == b.cache_write
        && a.cache_read == b.cache_read
}

/// Why a catalog could not be built.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// A price table (built-in or user-supplied) failed to parse.
    #[error(transparent)]
    Price(#[from] PriceError),
    /// The embedded `models.toml` failed to parse.
    #[error("failed to parse model table: {0}")]
    Models(String),
}

/// The merged model catalog: one row per id, built-in rows filled by
/// plugins, then overridden by the host's entries, then by a user price file
/// (in that order, by id).
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    rows: HashMap<String, ModelRow>,
    /// Plugin rows that were ignored, one line each; returned rather than
    /// printed so the caller decides how to show them.
    warnings: Vec<String>,
}

impl Catalog {
    /// Built-in rows only: the embedded `models.toml` merged with the
    /// embedded `prices.toml`, both copied from cox, which writes them only
    /// through its vendor pipeline.
    pub fn builtin() -> Result<Self, CatalogError> {
        let models = config::builtin_models().map_err(CatalogError::Models)?;
        let mut catalog = Self::default();
        catalog.overlay_models(&models, &RowSource::Builtin);
        catalog.overlay_prices(&PriceTable::embedded()?, &RowSource::Builtin);
        Ok(catalog)
    }

    /// [`Catalog::builtin`] filled by `plugins`' `[[models]]` rows, then
    /// overridden by the host's `models` entries, then by
    /// `user_prices_toml` (the contents of a `prices.toml`-shaped file —
    /// already read by the caller; this crate does no I/O of its own).
    /// Ignored plugin rows are listed in [`Catalog::warnings`].
    pub fn load(
        models: &[ProviderModel],
        plugins: &[PluginModels<'_>],
        user_prices_toml: Option<&str>,
    ) -> Result<Self, CatalogError> {
        Self::builtin()?.layered(models, plugins, user_prices_toml)
    }

    /// Every layer above the built-in one, split from [`Catalog::load`] so
    /// a test can start from a built-in layer it shaped itself.
    fn layered(
        mut self,
        models: &[ProviderModel],
        plugins: &[PluginModels<'_>],
        user_prices_toml: Option<&str>,
    ) -> Result<Self, CatalogError> {
        self.overlay_plugins(plugins);
        self.overlay_models(models, &RowSource::Config);
        if let Some(content) = user_prices_toml {
            self.overlay_prices(&PriceTable::parse(content)?, &RowSource::UserPrices);
        }
        Ok(self)
    }

    /// Plugin rows that were ignored — a changed built-in value or an id a
    /// lower plugin id already defined — one line each.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// What a running local server reports for `id`: the context
    /// it actually loaded and whether the model was trained for tool use.
    /// Overrides every other layer, because the loaded window is the one
    /// compaction must fit, whatever the model could support. `None`
    /// fields keep the row's value. The caller does the I/O; this crate
    /// only merges the numbers.
    pub fn overlay_served(&mut self, id: &str, context_window: Option<u32>, tools: Option<bool>) {
        let row = self.row_mut(id, &RowSource::Served);
        if let Some(window) = context_window {
            row.context_window = Some(window);
        }
        if let Some(tools) = tools {
            row.capabilities.tools = Some(tools);
        }
    }

    /// A model's row, if the catalog has one.
    pub fn get(&self, id: &str) -> Option<&ModelRow> {
        self.rows.get(id)
    }

    /// Every row, for diagnostics.
    pub fn rows(&self) -> impl Iterator<Item = &ModelRow> {
        self.rows.values()
    }

    /// `source` is used only when the row does not exist yet.
    fn row_mut(&mut self, id: &str, source: &RowSource) -> &mut ModelRow {
        self.rows
            .entry(id.to_string())
            .or_insert_with(|| ModelRow::new(id.to_string(), source.clone()))
    }

    /// A plugin only fills. A new id is taken whole, and plugins run
    /// in id order so the lower id keeps a contested new id. On an existing
    /// row a plugin sets only fields that are still `None`; a different
    /// value — above all a price, which would let a plugin rewrite what the
    /// ledger charges — is ignored with a warning.
    fn overlay_plugins(&mut self, plugins: &[PluginModels<'_>]) {
        let mut ordered: Vec<&PluginModels<'_>> = plugins.iter().collect();
        ordered.sort_by_key(|p| p.plugin);
        for plugin in ordered {
            for decl in plugin.models {
                self.overlay_plugin_model(plugin.plugin, decl);
            }
        }
    }

    fn overlay_plugin_model(&mut self, plugin: &str, decl: &PluginModel) {
        let price = decl
            .price
            .as_ref()
            .map(|p| plugin_price(plugin, &decl.id, p));
        let Some(row) = self.rows.get_mut(&decl.id) else {
            let mut row = ModelRow::new(decl.id.clone(), RowSource::Plugin(plugin.to_string()));
            row.context_window = decl.context_window;
            row.max_output = decl.max_output;
            row.price = price;
            self.rows.insert(decl.id.clone(), row);
            return;
        };
        if let RowSource::Plugin(owner) = &row.source {
            self.warnings.push(format!(
                "plugin {plugin} also defines model {}; plugin {owner}'s row is kept",
                decl.id
            ));
            return;
        }
        let mut conflicts = Vec::new();
        fill(
            &mut row.context_window,
            decl.context_window,
            "context window",
            &mut conflicts,
        );
        fill(
            &mut row.max_output,
            decl.max_output,
            "max output",
            &mut conflicts,
        );
        match (&row.price, price) {
            (None, price) => row.price = price,
            (Some(old), Some(new)) if !same_rates(old, &new) => conflicts.push("price"),
            _ => {}
        }
        for field in conflicts {
            self.warnings.push(format!(
                "plugin {plugin} tried to change the {field} of {}",
                decl.id
            ));
        }
    }

    fn overlay_model(&mut self, model: &ProviderModel, source: &RowSource) {
        let row = self.row_mut(&model.id, source);
        row.context_window = Some(model.context_window);
        if model.display_name.is_some() {
            row.display_name.clone_from(&model.display_name);
        }
        if !model.efforts.is_empty() {
            row.efforts = model.efforts.clone();
        }
        // Like `efforts`: an entry that declares nothing keeps the row's.
        let declared = Capabilities::declared_by(model);
        if let Some(param) = declared.reasoning_effort_param {
            row.capabilities.reasoning_effort_param = Some(param);
        }
        if let Some(images) = declared.images {
            row.capabilities.images = Some(images);
        }
    }

    fn overlay_models(&mut self, models: &[ProviderModel], source: &RowSource) {
        for model in models {
            self.overlay_model(model, source);
        }
    }

    fn overlay_prices(&mut self, prices: &PriceTable, source: &RowSource) {
        for price in prices.prices() {
            self.row_mut(&price.id, source).price = Some(price.clone());
        }
    }
}

/// Fill-only merge of one field: `new` lands only where `slot` is `None`;
/// a different value is recorded as a conflict under `name`.
fn fill<T: PartialEq>(
    slot: &mut Option<T>,
    new: Option<T>,
    name: &'static str,
    conflicts: &mut Vec<&'static str>,
) {
    match (slot.as_ref(), new) {
        (None, new) => *slot = new,
        (Some(old), Some(new)) if *old != new => conflicts.push(name),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_price_toml(id: &str, input: f64) -> String {
        format!(
            r#"[[model]]
id = "{id}"
input = {input}
output = 1.0
cache_write = 1.0
cache_read = 1.0
verified_on = "2026-01-01"
source_url = "https://example.test"
"#
        )
    }

    #[test]
    fn served_context_overrides_config_and_keeps_the_price() {
        let config = vec![ProviderModel {
            id: "claude-haiku-4-5".into(),
            context_window: 1_000,
            ..ProviderModel::default()
        }];
        let mut catalog = Catalog::load(&config, &[], None).expect("catalog");
        catalog.overlay_served("claude-haiku-4-5", Some(251_648), Some(true));
        let row = catalog.get("claude-haiku-4-5").expect("row");
        assert_eq!(row.context_window, Some(251_648));
        assert_eq!(row.capabilities.tools, Some(true));
        assert!(row.price.is_some(), "the served report touches no price");
        // A server that reports nothing leaves the row alone.
        catalog.overlay_served("claude-haiku-4-5", None, None);
        assert_eq!(
            catalog
                .get("claude-haiku-4-5")
                .and_then(|r| r.context_window),
            Some(251_648)
        );
    }

    fn decl(id: &str) -> PluginModel {
        PluginModel {
            id: id.into(),
            context_window: None,
            max_output: None,
            price: None,
        }
    }

    fn decl_price(input: f64) -> PluginPrice {
        PluginPrice {
            input,
            output: 1.0,
            cache_read: None,
            cache_write: None,
        }
    }

    #[test]
    fn plugin_cannot_override_builtin_price() {
        let builtin = Catalog::builtin().expect("builtin catalog");
        let before = builtin.get("claude-haiku-4-5").expect("haiku row").clone();
        let models = [PluginModel {
            price: Some(decl_price(0.001)),
            ..decl("claude-haiku-4-5")
        }];
        let plugins = [PluginModels {
            plugin: "jev",
            models: &models,
        }];
        let catalog = Catalog::load(&[], &plugins, None).expect("catalog");
        let row = catalog.get("claude-haiku-4-5").expect("haiku row");
        assert_eq!(row.price, before.price);
        assert_eq!(row.source, RowSource::Builtin);
        assert_eq!(
            catalog.warnings(),
            ["plugin jev tried to change the price of claude-haiku-4-5"]
        );
    }

    #[test]
    fn plugin_fills_missing_context_window() {
        let mut base = Catalog::builtin().expect("builtin catalog");
        let window = base
            .rows
            .get_mut("claude-haiku-4-5")
            .map(|r| r.context_window.take());
        assert!(window.is_some(), "the built-in layer has a haiku row");
        let models = [PluginModel {
            context_window: Some(64_000),
            max_output: Some(8_000),
            ..decl("claude-haiku-4-5")
        }];
        let plugins = [PluginModels {
            plugin: "jev",
            models: &models,
        }];
        let catalog = base.layered(&[], &plugins, None).expect("catalog");
        let row = catalog.get("claude-haiku-4-5").expect("haiku row");
        assert_eq!(row.context_window, Some(64_000));
        assert_eq!(row.max_output, Some(8_000));
        // Filled, not taken over: still a built-in row, and nothing to warn.
        assert_eq!(row.source, RowSource::Builtin);
        assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
    }

    #[test]
    fn plugin_row_for_a_new_id_is_taken_whole() {
        let models = [PluginModel {
            context_window: Some(64_000),
            price: Some(decl_price(0.042)),
            ..decl("acme-coder")
        }];
        let plugins = [PluginModels {
            plugin: "jev",
            models: &models,
        }];
        let catalog = Catalog::load(&[], &plugins, None).expect("catalog");
        let row = catalog.get("acme-coder").expect("plugin row");
        assert_eq!(row.context_window, Some(64_000));
        assert_eq!(row.source, RowSource::Plugin("jev".into()));
        assert_eq!(row.source.to_string(), "plugin:jev");
        let price = row.price.as_ref().expect("priced");
        // No cache rates declared: billed at the input rate, never below.
        assert_eq!(
            (price.input, price.cache_read, price.cache_write),
            (0.042, 0.042, 0.042)
        );
    }

    #[test]
    fn config_overrides_plugin_row() {
        let models = [PluginModel {
            context_window: Some(64_000),
            ..decl("acme-coder")
        }];
        let plugins = [PluginModels {
            plugin: "jev",
            models: &models,
        }];
        let config = vec![ProviderModel {
            id: "acme-coder".into(),
            context_window: 128_000,
            ..ProviderModel::default()
        }];
        let user = user_price_toml("acme-coder", 9.0);
        let catalog = Catalog::load(&config, &plugins, Some(&user)).expect("catalog");
        let row = catalog.get("acme-coder").expect("plugin row");
        assert_eq!(row.context_window, Some(128_000));
        // The user price file sits above the plugin layer too.
        assert_eq!(row.price.as_ref().expect("priced").input, 9.0);
        assert_eq!(row.source, RowSource::Plugin("jev".into()));
        assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
    }

    #[test]
    fn duplicate_plugin_model_lower_id_wins() {
        let high = [PluginModel {
            context_window: Some(1_000),
            ..decl("shared-model")
        }];
        let low = [PluginModel {
            context_window: Some(2_000),
            ..decl("shared-model")
        }];
        // Passed higher id first: the order the caller hands them in must
        // not decide the winner.
        let plugins = [
            PluginModels {
                plugin: "zeta",
                models: &high,
            },
            PluginModels {
                plugin: "alpha",
                models: &low,
            },
        ];
        let catalog = Catalog::load(&[], &plugins, None).expect("catalog");
        let row = catalog.get("shared-model").expect("plugin row");
        assert_eq!(row.context_window, Some(2_000));
        assert_eq!(row.source, RowSource::Plugin("alpha".into()));
        assert_eq!(
            catalog.warnings(),
            ["plugin zeta also defines model shared-model; plugin alpha's row is kept"]
        );
    }

    #[test]
    fn builtin_names_claude_sonnet_5_from_models_dev() {
        let catalog = Catalog::builtin().expect("builtin catalog");
        let row = catalog.get("claude-sonnet-5").expect("sonnet row");
        assert_eq!(row.display_name.as_deref(), Some("Claude Sonnet 5"));
    }

    #[test]
    fn a_config_entry_without_a_name_keeps_the_builtin_one_and_a_new_id_has_none() {
        let config = vec![
            ProviderModel {
                id: "claude-sonnet-5".into(),
                context_window: 555_000,
                ..ProviderModel::default()
            },
            ProviderModel {
                id: "qwen3-coder".into(),
                context_window: 32_768,
                ..ProviderModel::default()
            },
        ];
        let catalog = Catalog::load(&config, &[], None).expect("catalog");
        let sonnet = catalog.get("claude-sonnet-5").expect("sonnet row");
        assert_eq!(sonnet.display_name.as_deref(), Some("Claude Sonnet 5"));
        assert_eq!(
            catalog.get("qwen3-coder").expect("qwen row").display_name,
            None
        );
    }

    #[test]
    fn builtin_prices_and_windows_the_default_models() {
        let catalog = Catalog::builtin().expect("builtin catalog");
        let row = catalog.get("claude-haiku-4-5").expect("haiku row");
        assert_eq!(row.context_window, Some(200_000));
        assert!(row.price.is_some());
    }

    #[test]
    fn config_models_override_builtin_context_window_and_efforts() {
        let config = vec![ProviderModel {
            id: "claude-haiku-4-5".into(),
            context_window: 555_000,
            efforts: vec![Effort::Low, Effort::High],
            ..ProviderModel::default()
        }];
        let catalog = Catalog::load(&config, &[], None).expect("catalog");
        let row = catalog.get("claude-haiku-4-5").expect("haiku row");
        assert_eq!(row.context_window, Some(555_000));
        assert_eq!(row.efforts, vec![Effort::Low, Effort::High]);
        // The price layer is untouched by the config layer: still built-in.
        assert!(row.price.is_some());
    }

    #[test]
    fn user_price_file_overrides_both_builtin_and_config_layers() {
        let config = vec![ProviderModel {
            id: "claude-haiku-4-5".into(),
            context_window: 555_000,
            efforts: vec![],
            ..ProviderModel::default()
        }];
        let user = user_price_toml("claude-haiku-4-5", 42.0);
        let catalog = Catalog::load(&config, &[], Some(&user)).expect("catalog");
        let row = catalog.get("claude-haiku-4-5").expect("haiku row");
        // config's context_window still wins (the user file carries no
        // such field) — the user file only overrides price.
        assert_eq!(row.context_window, Some(555_000));
        assert_eq!(row.price.as_ref().expect("priced").input, 42.0);
    }

    #[test]
    fn user_price_for_a_model_absent_from_config_still_gets_a_row() {
        let user = user_price_toml("brand-new-model", 7.0);
        let catalog = Catalog::load(&[], &[], Some(&user)).expect("catalog");
        let row = catalog.get("brand-new-model").expect("new row");
        assert_eq!(row.context_window, None);
        assert_eq!(row.price.as_ref().expect("priced").input, 7.0);
    }

    #[test]
    fn config_entry_declares_the_reasoning_effort_param() {
        let config = vec![ProviderModel {
            id: "qwen3-coder".into(),
            context_window: 32_768,
            reasoning_effort: Some(true),
            ..ProviderModel::default()
        }];
        let catalog = Catalog::load(&config, &[], None).expect("catalog");
        let row = catalog.get("qwen3-coder").expect("qwen row");
        assert_eq!(row.capabilities.reasoning_effort_param, Some(true));
        // A built-in row declares nothing: Chat sends no effort for it.
        let haiku = catalog.get("claude-haiku-4-5").expect("haiku row");
        assert_eq!(haiku.capabilities.reasoning_effort_param, None);
    }

    #[test]
    fn empty_config_efforts_do_not_erase_the_builtin_efforts() {
        // `ProviderModel.efforts: []` means "any" (its own doc comment) —
        // a config override that doesn't mention efforts must not clear
        // whatever the built-in row already had.
        let config = vec![ProviderModel {
            id: "claude-haiku-4-5".into(),
            context_window: 555_000,
            efforts: vec![],
            ..ProviderModel::default()
        }];
        let builtin = Catalog::builtin().expect("builtin catalog");
        let builtin_efforts = builtin
            .get("claude-haiku-4-5")
            .expect("haiku row")
            .efforts
            .clone();
        let catalog = Catalog::load(&config, &[], None).expect("catalog");
        let row = catalog.get("claude-haiku-4-5").expect("haiku row");
        assert_eq!(row.efforts, builtin_efforts);
    }
}
