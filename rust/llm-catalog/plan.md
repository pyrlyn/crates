# llm-catalog

The model catalog for LLM agents: context windows, efforts, capabilities and prices merged from built-in rows, host config and a user price file.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T1.6 | todo | P2 | 2 | 0% | |

### T1.6. Adopt llm-catalog in cox (needs publication)

cox's `cox-models` drops its own copies of the catalog, the price table (see `done.md` T1.5) and re-exports them from this crate, with `cox-protocol`'s `Config` mapped onto `llm_wire::ProviderModel` (cox's `ProviderModel`, `Capabilities` and `effort_for` come from `llm-wire`) and the plugin manifest onto `PluginModels` in the host. Blocked on publication: cox can only depend on a released `llm-catalog` (which needs a released `llm-wire`), because a path dependency breaks cox CI. Done when cox builds against the published crate, its tests pass unchanged, and the moved code and data files are gone from `cox-models`.
