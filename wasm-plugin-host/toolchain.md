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
| extism | local | https://github.com/extism/extism | Loads and calls WebAssembly plugins |
| wasmtime | local | https://github.com/bytecodealliance/wasmtime | Only to turn on the `anyhow` feature extism 1.30 needs |
| serde | local | https://github.com/serde-rs/serde | Call inputs and outputs |
| serde_json | local | https://github.com/serde-rs/json | JSON on the plugin wire |
| sha2 | local | https://github.com/RustCrypto/hashes | SHA-256 package digest |
| thiserror | local | https://github.com/dtolnay/thiserror | `PluginError` |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
