# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| just | mise | Test recipe | https://github.com/casey/just |
| the package manager | see its README | Installs swarfr | https://github.com/pyrlyn |
| swarfr | the package manager | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| globset | local | https://github.com/BurntSushi/ripgrep/tree/master/crates/globset | Compile path-glob rules (`Edit(src/**)`) once |
| thiserror | local | https://github.com/dtolnay/thiserror | `RuleError` |
| proptest | local | https://github.com/proptest-rs/proptest | Property: adding a deny rule never weakens a decision |
| rstest | local | https://github.com/la10736/rstest | The decision table |

## The package manager

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
