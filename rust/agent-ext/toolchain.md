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
| serde | local | https://github.com/serde-rs/serde | Frontmatter deserialization |
| serde_json | local | https://github.com/serde-rs/json | Skill tool schema and output, hook payloads and verdicts |
| serde-saphyr | local | https://github.com/bourumir-wyngs/serde-saphyr | SKILL.md frontmatter (maintained; serde_yaml is deprecated) |
| thiserror | local | https://github.com/dtolnay/thiserror | Typed errors |
| async-trait | local | https://github.com/dtolnay/async-trait | The `Hook` trait |
| schemars | local | https://github.com/GREsau/schemars | JSON Schema of the `[hooks]` config types |
| tokio | local | https://github.com/tokio-rs/tokio | Spawn hook commands with a timeout |
| nix | local | https://github.com/nix-rust/nix | Kill a timed-out hook's process group |
| regex | local | https://github.com/rust-lang/regex | Hook `matcher` patterns |
| tempfile | local | https://github.com/Stebalien/tempfile | Skill directory fixtures in tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
