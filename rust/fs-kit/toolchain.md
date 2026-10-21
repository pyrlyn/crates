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
| dunce | local | https://gitlab.com/kornelski/dunce | Canonical paths without the Windows `\\?\` prefix |
| reflink-copy | local (feature `clone`) | https://github.com/cargo-bins/reflink-copy | Copy-on-write file clones with a copy fallback on macOS, Linux and Windows (`tree::clone_tree`) |
| tempfile | local (dev) | https://github.com/Stebalien/tempfile | Scratch directories in the `write` tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
