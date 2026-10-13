# Toolchain

Workspace pin: `rust-toolchain.toml` channel `1.99.0` (profile `minimal`, components `rustfmt` and `clippy`). Member crates also document mise; the root pin is the toolchain file.

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | rustup, via `rust-toolchain.toml` | Build | https://github.com/rust-lang/rust |
| cargo | rustup, via `rust-toolchain.toml` | Build, test, and `cargo metadata` | https://github.com/rust-lang/cargo |
| git | system package | `git-changed-paths` shells out to it | https://git-scm.com |
| just | mise | Test recipes in the member crates | https://github.com/casey/just |
| ketch | see its README | Installs swarfr for member justfiles | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of `target/` after tests | https://github.com/listepo/swarfr |

## cargo

Direct dependencies of the workspace members. Path crates are this repo.

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| cargo-changed-packages | local | https://github.com/pyrlyn/crates-packages | Affected workspace packages |
| file-backup | local | https://github.com/pyrlyn/crates-packages | Sibling `.bak-<unix-seconds>` copies |
| dunce | local | https://gitlab.com/kornelski/dunce | `fs-kit` canonical paths without the Windows `\\?\` prefix |
| git-changed-paths | local | https://github.com/pyrlyn/crates-packages | Change set against a base ref |
| path-gates | local | https://github.com/pyrlyn/crates-packages | Changed paths to gates |
| scoped-check | local | https://github.com/pyrlyn/crates-packages | Check commands for a change |
| telemetry-setup | local | https://github.com/pyrlyn/crates-packages | tracing setup with secret redaction |
| gettext-catalog | local | https://github.com/pyrlyn/crates-packages | gettext catalogs, plural rules and fallback |
| sqlite-change-feed | local | https://github.com/pyrlyn/crates-packages | Cross-process SQLite change feed |
| abi-drift | local | https://github.com/pyrlyn/crates-packages | Drift tests for generated bindings |
| keychain-secret | local | https://github.com/pyrlyn/crates-packages | Secrets from the environment or the OS keychain |
| anyhow | local | https://github.com/dtolnay/anyhow | Top-level errors of `scoped-check` |
| assert_cmd | local | https://github.com/assert-rs/assert_cmd | Tests that run the `scoped-check` binary |
| clap | local | https://github.com/clap-rs/clap | `scoped-check` CLI |
| determinator | local | https://github.com/guppy-rs/guppy/tree/main/determinator | Maps changed paths to affected packages |
| globset | local | https://github.com/BurntSushi/ripgrep/tree/master/crates/globset | Compile and match gate globs |
| guppy | local | https://github.com/guppy-rs/guppy | Cargo package graph |
| rstest | local | https://github.com/la10736/rstest | Parameterized tests in `file-backup` |
| serde | local | https://github.com/serde-rs/serde | Config types |
| serde_json | local | https://github.com/serde-rs/json | `scoped-check` JSON output |
| shlex | local | https://github.com/comex/rust-shlex | Shell-quoting substituted values |
| tempfile | local | https://github.com/Stebalien/tempfile | Throwaway repos and files in tests |
| thiserror | local | https://github.com/dtolnay/thiserror | Crate-local error enums |
| toml | local | https://github.com/toml-rs/toml | Gate and scoped-check config |
| tracing | local | https://github.com/tokio-rs/tracing | `telemetry-setup` subscriber |
| tracing-subscriber | local | https://github.com/tokio-rs/tracing | `telemetry-setup` formatters and filter |
| tracing-appender | local | https://github.com/tokio-rs/tracing | `telemetry-setup` rotating file writer |
| regex | local | https://github.com/rust-lang/regex | `telemetry-setup` credential patterns |
| opentelemetry | local | https://github.com/open-telemetry/opentelemetry-rust | `telemetry-setup` `otlp` feature |
| opentelemetry_sdk | local | https://github.com/open-telemetry/opentelemetry-rust | `telemetry-setup` `otlp` feature |
| opentelemetry-otlp | local | https://github.com/open-telemetry/opentelemetry-rust | `telemetry-setup` `otlp` feature: OTLP/HTTP exporter |
| tracing-opentelemetry | local | https://github.com/tokio-rs/tracing-opentelemetry | `telemetry-setup` `otlp` feature |
| polib | local | https://github.com/BrettDong/polib | `gettext-catalog` parses `.po` files |
| sys-locale | local | https://github.com/1Password/sys-locale | `gettext-catalog` reads the OS UI languages |
| unic-langid | local | https://github.com/zbraniecki/unic-locale | `gettext-catalog` language identifiers |
| diesel | local | https://github.com/diesel-rs/diesel | `sqlite-change-feed` connection and pragma query |
| libsqlite3-sys | local | https://github.com/rusqlite/rusqlite | Bundled SQLite in `sqlite-change-feed` tests |
| cbindgen | local | https://github.com/mozilla/cbindgen | `abi-drift` renders C headers |
| csbindgen | local | https://github.com/Cysharp/csbindgen | `abi-drift` renders C# bindings |
| similar | local | https://github.com/mitsuhiko/similar | Unified diffs in `abi-drift` and `change-preview` |
| toml | local | https://github.com/toml-rs/toml | Gate and scoped-check config; `keychain-secret` inline-secret check |
| keyring | local | https://github.com/open-source-cooperative/keyring-rs | `keychain-secret` reaches the OS keychain |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of `target/` after tests |

## Linkers

- **Windows (MSVC):** `rust-lld` through `.cargo/config.toml` (`[target.*-pc-windows-msvc] linker`). It ships with rustup, nothing to install.
- **Linux:** default linker (Rust >= 1.90 already links `x86_64-unknown-linux-gnu` with rust-lld); mold is not wired here because CI does not use mise.
- **macOS:** Apple's native linker (mold has no macOS support).
- **Cranelift:** not used. It needs a nightly toolchain and this repository is on stable.
