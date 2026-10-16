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
| figment | local | https://github.com/SergioBenitez/Figment | Config layering, provenance metadata (`parse-value` reads env values; `test` gives `Jail` to the tests) |
| toml_edit | local | https://github.com/toml-rs/toml | In-place edits that keep comments and layout; `serde` feature validates an edited file against the types |
| schemars | local | https://github.com/GREsau/schemars | JSON Schema generated from the config types |
| serde | local | https://github.com/serde-rs/serde | `Serialize` and `DeserializeOwned` bounds on the caller's types |
| serde_json | local | https://github.com/serde-rs/json | Rendering the schema, JSON overrides and leaf values |
| thiserror | local | https://github.com/dtolnay/thiserror | The crate's error type |
| tempfile | local | https://github.com/Stebalien/tempfile | Scratch directories in tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
