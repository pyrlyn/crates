// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! The model catalog: one pure crate mapping a model id to its context
//! window, max output, efforts, capabilities and price, merged from layers
//! that each override the last by id — built-in rows (embedded
//! `models.toml` and `prices.toml`) < plugin rows (fill-only) < the host's
//! configured [`ProviderModel`]s < a user-supplied price file — plus the one
//! per-wire effort rule ([`effort_for`], from `llm-wire`).
//!
//! It does no I/O beyond parsing an embedded or caller-supplied string: a
//! file on disk is the caller's job, and the host's own config types are
//! mapped onto [`ProviderModel`], [`PluginModels`] by the host. Both data
//! files have a JSON Schema ([`models_schema`], [`prices_schema`]) kept in
//! `schema/`.

mod catalog;
mod config;
mod price;

pub use catalog::{
    Catalog, CatalogError, ModelRow, PluginModel, PluginModels, PluginPrice, RowSource,
};
pub use config::{models_schema, prices_schema};
// The model entry, its capabilities and the effort rule are defined once in
// `llm-wire`, where the wires read them without this crate's tables.
pub use llm_wire::{
    Api, Capabilities, ProviderModel, WireEffort, effort_for, supports_adaptive_thinking,
};
pub use price::{Price, PriceError, PriceTable};
