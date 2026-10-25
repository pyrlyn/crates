# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| cargo | mise | `cargo metadata`, which `Graph::load` runs | https://github.com/rust-lang/cargo |
| just | mise | Test recipe | https://github.com/casey/just |
| the package manager | see its README | Installs swarfr | https://github.com/pyrlyn |
| swarfr | the package manager | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| cargo_metadata | local | https://github.com/oli-obk/cargo_metadata | Run and parse `cargo metadata` |
| thiserror | local | https://github.com/dtolnay/thiserror | `LoadError` |

## The package manager

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
