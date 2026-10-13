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
| anyhow | local | https://github.com/dtolnay/anyhow | Errors that name the file |
| file-backup | local | https://github.com/pyrlyn/crates | Copy the old config to `<name>.bak-<unix-seconds>` before an edit |
| jsonc-parser | local | https://github.com/dprint/jsonc-parser | Lossless syntax tree: edit our entry, keep every other byte |
| serde_json | local | https://github.com/serde-rs/json | Entry values and reading an entry back |
| tempfile | local | https://github.com/Stebalien/tempfile | Temp directories in tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
