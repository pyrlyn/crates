# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| git | system package | Runtime: the CLI `git-changed-paths` shells out to | https://git-scm.com |
| cargo | mise | Runtime: `cargo metadata` via `cargo-changed-packages` | https://github.com/rust-lang/cargo |
| bash | system package | Runtime: `scripts/bench.sh` | https://www.gnu.org/software/bash/ |
| just | mise | Test recipe | https://github.com/casey/just |
| ketch | see its README | Installs swarfr | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| git-changed-paths | local | https://github.com/pyrlyn/crates | Change set against a base ref |
| path-gates | local | https://github.com/pyrlyn/crates | Changed paths to gates |
| cargo-changed-packages | local | https://github.com/pyrlyn/crates | Affected workspace packages |
| clap | local | https://github.com/clap-rs/clap | CLI parsing |
| serde | local | https://github.com/serde-rs/serde | Config and JSON output |
| serde_json | local | https://github.com/serde-rs/json | `plan --json` |
| toml | local | https://github.com/toml-rs/toml | Config parsing |
| shlex | local | https://github.com/comex/rust-shlex | Shell-quoting substituted values |
| anyhow | local | https://github.com/dtolnay/anyhow | Top-level errors of the binary |
| assert_cmd | local | https://github.com/assert-rs/assert_cmd | Tests: run the binary |
| tempfile | local | https://github.com/Stebalien/tempfile | Tests: throwaway git repos |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
