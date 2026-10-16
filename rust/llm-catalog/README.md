# llm-catalog

A model catalog for LLM agents: for a model id, its context window, max output, efforts, capability flags and price. Rows are merged from layers that each override the last by id: built-in rows, plugin rows (fill-only), the host's configured entries, then a user price file. The model entry (`ProviderModel`), `Capabilities` and the per-wire effort rule (`effort_for`) are defined in `llm-wire` and re-exported here. The crate also holds a `PriceTable` that costs a `llm_wire::Usage`.

```rust
use llm_catalog::Catalog;

let catalog = Catalog::load(&[], &[], None)?;
let row = catalog.get("claude-sonnet-5");
```

The crate does no I/O beyond parsing an embedded or caller-supplied string. `models.toml` and `prices.toml` are embedded; JSON Schemas for both are in `schema/`.
