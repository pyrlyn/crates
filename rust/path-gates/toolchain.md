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
| globset | local | https://github.com/BurntSushi/ripgrep/tree/master/crates/globset | Compile and match gate globs (`literal_separator`, `**/x` at the root) |
| serde | local | https://github.com/serde-rs/serde | Derive the config types |
| toml | local | https://github.com/toml-rs/toml | Parse the gate config |
| thiserror | local | https://github.com/dtolnay/thiserror | Crate-local `Error` enum |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
