# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| just | mise | Test recipe | https://github.com/casey/just |
| ketch | see its README | Installs swarfr | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| llm-wire | local (path) | https://github.com/pyrlyn/crates | `Effort`, `ModelId` and `Usage` |
| figment | local | https://github.com/SergioBenitez/Figment | Parse `models.toml`, `prices.toml` and a user price file (the one owner is `config.rs`) |
| schemars | local | https://github.com/GREsau/schemars | JSON Schema of `models.toml` and `prices.toml` |
| serde | local | https://github.com/serde-rs/serde | Deserialize the TOML files, serialize rows |
| serde_json | local | https://github.com/serde-rs/json | Pretty-print the generated schemas |
| thiserror | local | https://github.com/dtolnay/thiserror | `CatalogError` and `PriceError` |
