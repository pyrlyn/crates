# AGENTS.md

If an AGENTS.md or CLAUDE.md exists higher in the tree, follow it too; on conflict, ask the creator.

## What this workspace is

Cargo workspace of five small Rust crates shared by ketch and rtok:

| Crate | Role |
| --- | --- |
| `abi-drift` | Drift tests for generated bindings: regenerate a cbindgen header or csbindgen C# file, diff it against the committed copy, bless to rewrite |
| `cargo-changed-packages` | Workspace packages a set of changed paths affects, including reverse dependencies |
| `file-backup` | Copy a file to `<name>.bak-<unix-seconds>` beside it before replacing it |
| `gettext-catalog` | gettext `.po` catalogs for an application's own strings: plural rules, named placeholders, language negotiation and per-message fallback |
| `git-changed-paths` | Paths a git working tree changed relative to a base ref |
| `keychain-secret` | Application secrets from an environment variable or the OS keychain, never a config file, with a test guard against the real keychain |
| `path-gates` | Map changed paths to named gates by glob rules from a TOML config |
| `plugwright` | Plugin manager for Rust hosts (native and Wasm); a separate workspace excluded from this one, with its own `Cargo.lock`, `rust-toolchain.toml` (1.97.1) and `AGENTS.md`; CI job `plugwright` |
| `scoped-check` | Binary that runs only the check commands a change touches |
| `telemetry-setup` | `tracing` setup with rotating JSON logs, optional OTLP traces and secret redaction, shared by aulo and Mailune |
| `sqlite-change-feed` | Change feed for a SQLite database shared between processes: `PRAGMA data_version` through Diesel |
| `wasm-plugin-host` | WebAssembly plugin host (extism) shared by cox and scull: worker per plugin, memory cap, deadlines |

Each member directory has its own `AGENTS.md`. This file is the workspace root.

`Cargo.toml` `repository` fields and the root README name
`https://github.com/pyrlyn/crates`. This checkout's `origin` is
`git@github.com:pyrlyn/crates.git`.

## Commands

From the workspace root:

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

Rust is pinned by `rust-toolchain.toml` (channel 1.99.0, with rustfmt and clippy).

jactionlint, hk and shellcheck are pinned in `mise.toml` (lint tools only). `.github/jactionlint.yaml`
applies the default profile and the baseline `.github/jactionlint-baseline.json`. `hk.pkl` runs
jactionlint as a pre-push hook:

```bash
mise x -- jactionlint                  # lint workflows, composite actions, dependabot.yml
mise x -- jactionlint --baseline-write # refresh the baseline after fixing findings
mise x -- hk install --mise            # install the pre-push hook once per clone
```
Releases are described in the README: `release-plz` updates versions, and only
`file-backup` is published to crates.io.

## Tests

`file-backup` keeps its tests in `src/lib.rs` under `#[cfg(test)]` (missing
file, identical-byte skip, and the suffix used when the timestamp collides).
The other four crates use a `tests/` directory. Do not add a second harness
for `file-backup` while those inline tests are still the suite.
