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
| cbindgen | local | https://github.com/mozilla/cbindgen | Renders the C header (`cbindgen` feature) |
| csbindgen | local | https://github.com/Cysharp/csbindgen | Renders the C# file (`csbindgen` feature) |
| similar | local | https://github.com/mitsuhiko/similar | Unified diff of committed and generated text |
| thiserror | local | https://github.com/dtolnay/thiserror | `Error` |
| tempfile | local | https://github.com/Stebalien/tempfile | Scratch copies in tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
