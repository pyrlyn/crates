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
| diesel | local | https://github.com/diesel-rs/diesel | `SqliteConnection` and the pragma query |
| thiserror | local | https://github.com/dtolnay/thiserror | `Error` |
| libsqlite3-sys | local | https://github.com/rusqlite/rusqlite | Bundled SQLite for the tests only |
| tempfile | local | https://github.com/Stebalien/tempfile | Database files in tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
