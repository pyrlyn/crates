# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| cargo | mise | Runs `cargo metadata` for the package graph | https://github.com/rust-lang/cargo |
| just | mise | Test recipe | https://github.com/casey/just |
| ketch | see its README | Installs swarfr | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| guppy | local | https://github.com/guppy-rs/guppy | Cargo package graph (0.17 line: determinator 0.12 requires it) |
| determinator | local | https://github.com/guppy-rs/guppy/tree/main/determinator | Maps changed paths to affected packages |
| thiserror | local | https://github.com/dtolnay/thiserror | Error enum |
| tempfile | local | https://github.com/Stebalien/tempfile | Tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
