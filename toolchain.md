# Toolchain

Workspace pin: `rust-toolchain.toml` channel `1.99.0` (profile `minimal`, components `rustfmt`, `clippy` and `rust-src`). `mise.toml` pins the same Rust with the same components; CI installs it from there.

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| sccache | mise (`mise.toml`, Linux and macOS) | Optional compiler cache; shared CI uses Cloudflare R2 when configured | https://github.com/mozilla/sccache |
| mold | mise (`mise.toml`, Linux only) | Faster linking through clang when installed | https://github.com/rui314/mold |
| rustc | rustup, via `rust-toolchain.toml` | Build | https://github.com/rust-lang/rust |
| cargo | rustup, via `rust-toolchain.toml` | Build, test, and `cargo metadata` | https://github.com/rust-lang/cargo |
| git | system package | `git-changed-paths` shells out to it | https://git-scm.com |
| just | mise | Test recipes in the member crates | https://github.com/casey/just |
| ketch | see its README | Installs swarfr for member justfiles | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of `target/` after tests | https://github.com/listepo/swarfr |
| jactionlint | mise (`mise.toml` pins `github:jdx/jactionlint` 2.0.2) | Lints the workflows (CI, the pre-push hook and `mise x -- jactionlint`) | https://github.com/jdx/jactionlint |
| hk | mise (`mise.toml` pins 2.5.0) | Runs jactionlint as a pre-push hook (`hk.pkl`) | https://github.com/jdx/hk |
| shellcheck | mise (`mise.toml` pins 0.11.0) | Checks `run:` scripts inside jactionlint | https://github.com/koalaman/shellcheck |

## cargo

Direct dependencies of the workspace members. Path crates are this repo.

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| cargo-changed-packages | local | https://github.com/pyrlyn/crates | Affected workspace packages |
| file-backup | local | https://github.com/pyrlyn/crates | Sibling `.bak-<unix-seconds>` copies |
| dunce | local | https://gitlab.com/kornelski/dunce | `fs-kit` canonical paths without the Windows `\\?\` prefix |
| git-changed-paths | local | https://github.com/pyrlyn/crates | Change set against a base ref |
| path-gates | local | https://github.com/pyrlyn/crates | Changed paths to gates |
| scoped-check | local | https://github.com/pyrlyn/crates | Check commands for a change |
| telemetry-setup | local | https://github.com/pyrlyn/crates | tracing setup with secret redaction |
| gettext-catalog | local | https://github.com/pyrlyn/crates | gettext catalogs, plural rules and fallback |
| sqlite-change-feed | local | https://github.com/pyrlyn/crates | Cross-process SQLite change feed |
| abi-drift | local | https://github.com/pyrlyn/crates | Drift tests for generated bindings |
| keychain-secret | local | https://github.com/pyrlyn/crates | Secrets from the environment or the OS keychain |
| anyhow | local | https://github.com/dtolnay/anyhow | Top-level errors of `scoped-check` |
| assert_cmd | local | https://github.com/assert-rs/assert_cmd | Tests that run the `scoped-check` binary |
| clap | local | https://github.com/clap-rs/clap | CLI trees and shared completion generation |
| clap_complete | local | https://crates.io/crates/clap_complete | Standard shell completion generators |
| determinator | local | https://github.com/guppy-rs/guppy/tree/main/determinator | Maps changed paths to affected packages |
| globset | local | https://github.com/BurntSushi/ripgrep/tree/master/crates/globset | Compile and match gate globs |
| guppy | local | https://github.com/guppy-rs/guppy | Cargo package graph |
| rstest | local | https://github.com/la10736/rstest | Parameterized tests in `file-backup` |
| same-file | local | https://github.com/BurntSushi/same-file | Hard-link check on every platform in `file-backup` |
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
| tokio | local | https://github.com/tokio-rs/tokio | `owner-listener` async sockets and pipes |
| tokio-stream | local | https://github.com/tokio-rs/tokio | `owner-listener` incoming-connection stream for tonic |
| tokio-util | local | https://github.com/tokio-rs/tokio | `owner-listener` cancellation token that stops the pipe accept loop |
| tonic | local | https://github.com/hyperium/tonic | `owner-listener` `Connected` impls |
| rustix | local | https://github.com/bytecodealliance/rustix | `owner-listener` peer credentials and modes on Unix |
| windows-sys | local | https://github.com/microsoft/windows-rs | `owner-listener` pipe DACL, token and client pid on Windows |
| cbindgen | local | https://github.com/mozilla/cbindgen | `abi-drift` renders C headers |
| csbindgen | local | https://github.com/Cysharp/csbindgen | `abi-drift` renders C# bindings |
| similar | local | https://github.com/mitsuhiko/similar | Unified diffs in `abi-drift` and `change-preview` |
| toml | local | https://github.com/toml-rs/toml | Gate and scoped-check config; `keychain-secret` inline-secret check |
| keyring | local | https://github.com/open-source-cooperative/keyring-rs | `keychain-secret` reaches the OS keychain |

## mise

`mise.toml` pins the lint tools only; Rust stays on `rust-toolchain.toml`.

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| github:jdx/jactionlint | global | https://github.com/jdx/jactionlint | Workflow lint |
| hk | global | https://github.com/jdx/hk | Pre-push hook runner |
| shellcheck | global | https://github.com/koalaman/shellcheck | Shell checks inside jactionlint |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of `target/` after tests |

## Linkers

- **Windows (MSVC):** `rust-lld` through `.cargo/config.toml` (`[target.*-pc-windows-msvc] linker`). It ships with rustup, nothing to install.
- **Linux:** mise selects mold through clang when mold is installed; otherwise the default linker remains (Rust >= 1.90 already uses rust-lld for `x86_64-unknown-linux-gnu`).
- **macOS:** Apple's native linker (mold has no macOS support).
- **Cranelift:** opt-in dev builds through `cargo +nightly --config .cargo/cranelift.toml build`; the stable default and release builds keep LLVM.

## Compiler cache

`mise exec` selects the installed sccache as `RUSTC_WRAPPER` on Linux and macOS; a missing tool leaves the wrapper empty. The shared Rust CI workflow uses the optional R2 cache configuration and reads the cache on pull requests. Without its backend settings, CI continues without the remote cache.
