# crates

<https://github.com/pyrlyn/crates>

Cargo workspace of five small published crates shared by ketch and rtok — git-changed-paths, path-gates, cargo-changed-packages, scoped-check, file-backup — released via release-plz.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T36 | in progress | P1 | 2 | 0% | OpenAI / GPT-6 |
| T1 | todo | P1 | 3 | 0% | |
| T2 | todo | P1 | 2 | 0% | |
| T4 | todo | P2 | 2 | 0% | |
| T5 | todo | P3 | 1 | 0% | |
| T6 | todo | P2 | 2 | 0% | |
| T7 | todo | P3 | 2 | 0% | |
| T8 | todo | P3 | 1 | 0% | |
| T9 | todo | P3 | 2 | 0% | |
| T10 | todo | P2 | 1 | 0% | |
| T13 | in progress | P1 | 2 | 90% | Claude / opus-5.5 |
| T20.2 | done | P1 | 3 | 100% | |
| T20.3 | todo | P2 | 4 | 0% | |
| T21 | in progress | P1 | 3 | 90% | Cursor / claude-opus-5.5 |
| T22.1 | in progress | P1 | 2 | 90% | Cursor / claude-opus-5.5 |
| T22.2 | in progress | P1 | 2 | 90% | Cursor / claude-opus-5.5 |
| T23 | in progress | P1 | 2 | 90% | Cursor / claude-opus-5.5 |
| T24 | in progress | P1 | 2 | 90% | Cursor / claude-opus-5.5 |
| T25 | in progress | P1 | 2 | 90% | Cursor / claude-opus-5.5 |

Audit note (2026-10-07): verified defenses — git argument injection refused, POSIX shell quoting correct, path traversal blocked, fail-safe direction is always "everything changed". The tasks below are what remains.

### T1. Windows: POSIX quoting handed to cmd.exe in scoped-check

`rust/scoped-check/src/plan.rs:139,155-157` builds `{changed}`/`{packages}` with `shlex::try_join`/`try_quote` (POSIX single quotes), but `rust/scoped-check/src/main.rs:146-149` runs commands via `cmd /C` on Windows, which does not honor them. A file named `docs/x & calc & .md` executes `calc`; plain spaces break arguments. Tests are `#![cfg(unix)]` (`rust/scoped-check/tests/cli.rs:4`), so the path is untested. Done means: Windows-appropriate quoting (or raw argument passing) with tests covering it.

### T2. `{changed}` silently expands to empty in full mode

`rust/scoped-check/src/plan.rs:87-91` sets `changed: String::new()` for `--all`, unmatched-path fallback, and error fallbacks, while `paths` is usually already known (`plan.rs:58-63`). A gate `run = "gofmt -w {changed}"` becomes `gofmt -w` (reads stdin, hangs CI); test `cli.rs:176-180` pins the trailing-empty behavior. Done means: the full quoted path list is substituted whenever paths are known, and a forced plan that still contains `{changed}` with unknown paths warns or refuses.

### T4. file-backup: name-probe race and symlink-following overwrite

`rust/file-backup/src/lib.rs:39-45` does `while bak.exists()` then `fs::copy` — two concurrent backups can pick the same name and the second silently truncates the first (exactly what the comment claims to prevent), and a planted `<name>.bak-<ts>` dangling symlink is followed by `fs::copy`. Done means: the destination is created with `OpenOptions::create_new(true)` + `io::copy`, bumping the suffix on `AlreadyExists`.

### T5. Error-quality fixes: canonicalize and NotARepository

`rust/cargo-changed-packages/src/lib.rs:103` compares two `canonicalize().ok()` results — double failure passes as equality; bail when either errors. `rust/git-changed-paths/src/lib.rs:78-86` reports "not a git repository" for a nonexistent directory; distinguish the two cases.

### T6. Deduplicate git-toplevel discovery

Repo-top discovery exists privately in `rust/git-changed-paths/src/lib.rs:78-86` and again in `rust/scoped-check/src/main.rs:94-103` (losing `GIT_OPTIONAL_LOCKS=0` and the error mapping); the same pattern is copy-pasted in cox (`crates/cox-tools/src/git.rs:518`) and swarfr (`src/session.rs:520`). Done means: `git_changed_paths::toplevel(dir)` is public and all four call sites use it (coordinate the cox/swarfr swap with those projects).

### T7. Extract the shared test fixtures

The "a depends on b, c unrelated" workspace fixture is built twice (`rust/cargo-changed-packages/tests/affected.rs:13-34`, `rust/scoped-check/tests/cli.rs:55-92`) and the `git()` helper twice (`rust/git-changed-paths/tests/changed_paths.rs:11-26`, `cli.rs:38-47`). Done means: one dev-only shared test-support crate in this workspace.

### T8. Parse the scoped-check config TOML once

`Config::parse` parses the same TOML text twice with two schemas (`rust/scoped-check/src/config.rs:73-74`). Done means: one struct embedding `GateSpec` parses once, keeping the acknowledged trade-offs in mind.

### T9. Robustness batch: dead gates, dropped paths, `./` stripping

A `[[gate]]` with `run` but no `paths`/`always` is silently unselectable (`rust/path-gates/src/lib.rs:121-131`) — a typo'd glob skips checks; warn or add `plan --lint`. `Selection::unmatched` is never surfaced when `unmatched = "ignore"` (`plan.rs:71-84`). `rust/path-gates/src/lib.rs:199-202` strips only one leading `./` (`././x` stays `./x`, breaking exact patterns like `Cargo.lock`). Done means: all three fixed with tests.

### T10. Fix crate metadata: license mix and repository URL drift

Workspace `Cargo.toml` sets `license = "GPL-3.0-or-later"` (inherited only by file-backup) while the other four crates hardcode `MIT`; every `repository` field, README badges, and the release-plz postprocessor say `pyrlyn/crates` while origin is `pyrlyn/crates`. Done means: one license everywhere (intentionally chosen) and URLs match the canonical remote.

### T20.2. wasm-plugin-host: package layout

Plugin packages on disk, generic over the application's manifest type and home directory: discovery of user plugins under `<home>/plugins/<id>/versions/<digest12>/` (following `current`) and project plugins under `<root>/.<app>/plugins/<id>/`, the package digest, staging a package and the `current`/`previous` swap, `link` for development, and `remove` confined to the plugins root. Ported from cox `crates/cox-plugin/src/discover.rs` and `install.rs`; coordinate with roadmap T16 `dir-ops`, which also draws on `install.rs`. Done when both are in the crate with their tests.

### T20.3. wasm-plugin-host: host-function kit

What every application's host functions repeat: the `(u64) -> u64` JSON wire with an `{"Ok": …}` / `{"Err": …}` reply, the per-export refusal rule, the plugin key-value store with its quotas, and the outbound HTTP allow-list with its body cap. Ported from cox `crates/cox-plugin/src/hostfn.rs` and `net.rs`, leaving the cox-only functions (context, tools, model calls) in cox. Done when cox's kernel functions could be rebuilt on the kit with their tests passing here.

### T21. telemetry-setup: tracing setup with secret redaction

Approved by the creator as Mailune's X3 (2026-10-08: code Mailune shares with other projects is extracted here). aulo (`aulo-telemetry`) and cox (`cox-telemetry`) each set up `tracing` by hand: a rotating JSON log file, a filter from config or an environment variable, and optional OTLP export. Only aulo masks secrets before a line reaches a sink. Mailune needs the same (its F5). rtok `src/otel` exports rtok's own events and has nothing to share. New crate `telemetry-setup`, extracted from `aulo-telemetry` with no change in what is masked. Done means: the crate is in the workspace with aulo's redaction and logging tests, registered for CI dry-run publish, bump, README and sonar. Consumers migrate in their own tasks once the crate is published (aulo first, then Mailune F5).

Plan:
1. `rust/telemetry-setup/` with the member files of `wasm-plugin-host`. Edition 2024, `rust-version` 1.98 (cox, the lowest candidate consumer). Dependency versions are the ones aulo locks, so neither lock moves.
2. From `aulo-telemetry`: `redact` (masking of a finished line, structural masking of a JSON record), the buffering `Scrubbed` writer, `Settings`/`subscriber`/`init`, and the `otlp` feature that masks span attributes before export.
3. What named aulo becomes a setting: the application name (file prefix and OTLP service name), the filter environment variable, and extra credential patterns (aulo's `aulo_<hex>` tokens). An invalid pattern or an application name that is not a plain file name is an error at setup.
4. Register: workspace members, `ci.yml` dry-run publish and an `otlp` feature test on Linux, `bump.yml` package option, root README crate table and token scope, `sonar-project.properties`, root `toolchain.md`.
5. Check: `cargo test --workspace --locked`, the same with `-p telemetry-setup --features otlp`, clippy `-D warnings` with and without the feature, fmt, `cargo publish --dry-run -p telemetry-setup --locked`, `cargo +1.98 check -p telemetry-setup --all-features`.

Checked locally on 2026-10-10 (fmt, clippy with and without `otlp`, workspace tests, the `otlp` tests, `cargo +1.98 check --all-features`, `cargo publish --dry-run -p telemetry-setup --locked`): all pass. The package ships only `src`, `tests` and `README.md`.

Left: review and merge; GitHub Actions is disabled on this repository, so neither CI nor `bump.yml` can run until it is enabled; the `CARGO_REGISTRY_TOKEN` scope must add `telemetry-setup` before its first `bump.yml` run. Consumers migrate once the crate is on crates.io (`rust.md`: a registry version plus a local `paths` override, never a bare path).

### T13. app-home: one home, app-home and XDG resolver

About 30 hand-written resolvers across rtok, cox, aulo, ketch, runa and swarfr disagree on empty variables, the Windows `USERPROFILE` fallback, relative XDG values and the no-home fallback (runa alone has 9; cox and ketch each carry two that disagree). New crate `app-home` (std only, edition 2021, Rust 1.86). Done means: the crate is in the workspace with tests and registered like `atomic-replace`; consumers migrate in their own tasks.

Plan:
1. `Dirs` over an injectable variable lookup (`Dirs::from_env()`, `Dirs::with(lookup)`), plus free functions over the process env.
2. `user_home`: non-empty `HOME`, else non-empty `USERPROFILE`, else `std::env::home_dir` (process env only). From rtok-hook `user_home_from` and cox `home_dir`.
3. `app_home(var, dot_name)`: non-empty `$var` with `~` expanded, else `<user home>/<dot_name>`. From rtok-hook `home_dir_from`, cox `cox_home`, aulo `aulo_home`.
4. `config_home`/`data_home`/`cache_home`/`state_home`: the XDG variable when absolute (the spec says relative values are invalid), else `~/.config`, `~/.local/share`, `~/.cache`, `~/.local/state` on every OS. From ketch `platform`/`shell.rs` and runa.
5. `expand_tilde(path, home)`: `~`, `~/` and `~\` joined by components. From ketch `config.rs` and rtok-hook `join_tilde_rest`.
6. Check: tests with injected env, clippy for macOS/Linux/Windows, Rust 1.86, publish dry-run.

### T22.1. gettext-catalog: catalog parsing, plural rules and placeholders

Approved by the creator as Mailune's X4 (2026-10-10: extract the shared code and publish it through release-plz). cox's `cox-i18n` parses `.po` catalogs with `polib`, evaluates `Plural-Forms` with its own parser, and fills `{name}` placeholders; Mailune's F10 needs the same for core-originated strings. New crate `gettext-catalog`, split into two tasks to stay under 500 lines each. This one: `catalog` (entries by key, fuzzy and incomplete plurals untranslated, the `# cldr-other:` override), `plural` (the GNU gettext C subset, tested against CLDR for ru and uk) and `format` (`pieces`, `placeholders`, `render`, `validate`), with one `Error` enum for the crate. Done means: the crate is in the workspace with those modules and their tests, registered for CI dry-run publish, bump, README, sonar and the root `toolchain.md`.

Plan:
1. `rust/gettext-catalog/` with the member files of `wasm-plugin-host`. Edition 2024, `rust-version` 1.98 (cox). Dependency versions are the ones cox locks (`polib 0.3.0`, `unic-langid 0.9.6`, `sys-locale 0.3.2`).
2. Port `catalog.rs`, `plural.rs` and `format.rs` from `cox-i18n` with their tests; the plural and placeholder errors become variants of the crate's `Error`, and the evaluator loses its `unreachable!`.
3. Register as T21 did; the package ships only `src`, `tests` and `README.md`.
4. Check: fmt, clippy `-D warnings`, `cargo test --workspace --locked`, `cargo publish --dry-run -p gettext-catalog --locked`.

### T22.2. gettext-catalog: localizer with negotiation and fallback

The second half of Mailune's X4. `Localizer` over the locales an application passes in (code, `.po` text, CLDR `other` form) and a default locale: negotiation by language subtag from POSIX and OS tags (`uk_UA.UTF-8`, `ru-RU`), the user's languages from `LC_ALL`, `LC_MESSAGES`, `LANG` and `sys-locale`, per-message fallback (translation, default `msgstr`, source text, then the id itself), `count` selecting the plural form, and constants such as a product name (cox's `{brand}`). cox keeps its embedded catalogs, `global()`, `tr!` and the native-catalog export. Done means: the localizer is in the crate with `.po` fixtures for en, ru and uk and tests for plural tables, fractional and text counts, fallback, negotiation and placeholders.

Plan:
1. `src/lib.rs`: `Locale`, `Value`, `Args`, `Localizer` (`new`, `for_tags`, `from_env`, `with_constant`, `chain`, `try_format`, `format`), `negotiate`, `parse_tag`, `requested_languages`, ported from `cox-i18n` with the cox constants (`LOCALES`, `BRAND_NAME`, `DEFAULT_LOCALE`) turned into parameters.
2. `tests/fixtures/{en,ru,uk}.po` and `tests/localizer.rs`.
3. Check as T22.1, plus the doc example.

Left (both): review and merge; GitHub Actions must be enabled on this repository and `CARGO_REGISTRY_TOKEN` must cover `gettext-catalog` before its first `bump.yml` run. cox and Mailune migrate once the crate is on crates.io.

### T23. sqlite-change-feed: cross-process SQLite change feed

Approved by the creator as Mailune's X11 (2026-10-10: extract the shared code and publish it through release-plz). cox's `cox-store` (`src/watch.rs`) tells the app and a TUI sharing one `cox.db` when the other committed, by polling `PRAGMA data_version`; Mailune's S7 change feed reads the same pragma. New crate `sqlite-change-feed`: `ChangeToken::new(conn)`, `ChangeToken::poll(conn)` and `data_version(conn)` over a Diesel `SqliteConnection`, with no connection, thread or `libsqlite3-sys` choice of its own. Done means: the crate is in the workspace with a test in which a writer in another process commits and the feed sees it once, plus a second-connection test in the same process, registered for CI dry-run publish, bump, README, sonar and the root `toolchain.md`.

Plan:
1. `rust/sqlite-change-feed/` with the member files of `wasm-plugin-host`. Edition 2024, `rust-version` 1.98 (cox). `diesel 2.3.13` with only the `sqlite` feature (cox and Mailune lock 2.3.13 and 2.3.14); bundled `libsqlite3-sys 0.38.2` as a dev-dependency only.
2. Port the token and the pragma from `watch.rs`; the store's mutex and `StoreError` stay in cox.
3. `tests/feed.rs`: the cross-process test re-executes the test binary as cox's does (an ignored `writer_process` test), plus own-commit, second-connection and two-consumer tests.
4. Register as T21 did; the package ships only `src`, `tests` and `README.md`.
5. Check: fmt, clippy `-D warnings`, `cargo test --workspace --locked`, `cargo nextest run -p sqlite-change-feed`, `cargo publish --dry-run -p sqlite-change-feed --locked`.

Left: review and merge; GitHub Actions must be enabled on this repository and `CARGO_REGISTRY_TOKEN` must cover `sqlite-change-feed` before its first `bump.yml` run. cox and Mailune migrate once the crate is on crates.io.

### T24. abi-drift: drift tests for generated bindings

Approved by the creator as Mailune's X9 (2026-10-10: extract the shared code and publish it through release-plz). scull (`crates/scull-ffi/tests/bindings.rs`) and ketch (`ketch-capi`'s header test) each regenerate their cbindgen header (and scull its csbindgen C# file) in a test, compare with the committed copy and rewrite it under `SCULL_BLESS` / `KETCH_BLESS`; Mailune's B6 C ABI needs the same. New crate `abi-drift`: `Drift` (compare with LF line endings, a unified diff on drift, bless from a flag or a named variable, the regenerate command in the message) and `cbindgen_header` / `csbindgen_file` behind default features. Done means: the crate is in the workspace with a fixture FFI crate whose committed header and C# file are checked, and tests for a stale copy showing its diff, blessing, a missing copy and CRLF, registered for CI dry-run publish, bump, README, sonar and the root `toolchain.md`.

Plan:
1. `rust/abi-drift/` with the member files of `wasm-plugin-host`. Edition 2021, `rust-version` 1.86 (ketch). `cbindgen 0.29.4` (library only) and `csbindgen 1.9.8` as optional default features, as scull and ketch lock them; `similar 3.2` for the diff, as `change-preview` uses it.
2. The shared `check_drift` of both tests becomes `Drift::check`, returning `Error::Stale` with the diff instead of panicking; `Debug` prints the same text so a test's `?` shows it.
3. `tests/fixtures/ffi` (source, `cbindgen.toml`, committed header and C# file, regenerated with `ABI_DRIFT_BLESS=1`) and `tests/drift.rs`.
4. Register as T21 did; the package ships only `src`, `tests` and `README.md`.
5. Check: fmt, clippy `-D warnings` with and without default features, `cargo test --workspace --locked`, `cargo publish --dry-run -p abi-drift --locked`, `cargo +1.86 check -p abi-drift`.

Left: review and merge; GitHub Actions must be enabled on this repository and `CARGO_REGISTRY_TOKEN` must cover `abi-drift` before its first `bump.yml` run. scull, ketch and Mailune migrate once the crate is on crates.io.

### T25. keychain-secret: secrets from the environment or the OS keychain

Approved by the creator as Mailune's X2 (2026-10-10: extract the shared code and publish it through release-plz; the crates.io name `secret-store` belongs to someone else, so the crate is `keychain-secret`). runa (`runa-cloud/src/secrets.rs`), cox (`cox-provider-http` `resolve_key_with` and its `no_real_keychain_in_tests` guard) and aulo (`aulo-server/src/auth/token.rs`) each resolve a secret from an environment variable, then the keyring, keep it out of logs, and keep tests off the real keychain; runa also refuses inline keys in config files. Mailune's C1 needs all of it. New crate `keychain-secret`. Done means: one implementation in the workspace with a redacted `Secret`, a `SecretStore` trait with `Keychain`, `MemoryStore` and `NoStore`, `resolve` (env, then store), the keychain switch, `reject_inline_secrets`, and the no-real-keychain `guard` scanner, with tests that never touch the keychain, registered for CI dry-run publish, bump, README, sonar and the root `toolchain.md`.

Plan:
1. `rust/keychain-secret/` with the member files of `wasm-plugin-host`. Edition 2024, `rust-version` 1.98 (cox, runa). `keyring 4.2.0`, `toml 0.9.12`, as the consumers lock them.
2. From aulo: the redacted secret type and the store trait with keychain and memory stores (keyring errors keep only their message). From cox and runa: env-then-store resolution with an injectable lookup, returning `None` so each caller decides what a missing key means, and the `COX_KEYRING=off` switch as `keychain_enabled`/`os_store`. From runa: `reject_inline_secrets`, plus `password`, `client_secret` and `refresh_token`. From cox: the guard scanner, walking any repository root with caller-given patterns; it also treats `'"'` as a character, not a string start.
3. Register as T21 did; the package ships only `src` and `README.md`.
4. Check: fmt, clippy `-D warnings`, `cargo test --workspace --locked`, `cargo publish --dry-run -p keychain-secret --locked`.

Left: review and merge; GitHub Actions must be enabled on this repository and `CARGO_REGISTRY_TOKEN` must cover `keychain-secret` before its first `bump.yml` run. The first consumer (Mailune C1, or runa) migrates once the crate is on crates.io.


### T36. Publish MIT-licensed shell-completions 0.1.1

The published 0.1.0 release is GPL-3.0-or-later and conflicts with downstream consumers' explicit license allowlist. Publish version 0.1.1 under MIT so applications can adopt it while preserving their license guard. Execution plan: update only the shell-completions crate manifest and license header/text; verify the crate archive includes the MIT license and normalized manifest, preserving every unrelated lockfile package; run its existing tests and formatting; run `cargo publish --dry-run -p shell-completions --locked`; commit and open a PR for managed publication. Do not publish manually. Done when the managed pipeline publishes 0.1.1 and downstream consumers resolve it.

Validation: the existing crate test suite passed (4/4), `cargo clippy -p shell-completions --all-targets --locked -- -D warnings` passed, `cargo fmt --all -- --check` passed, and `cargo publish --dry-run -p shell-completions --locked --allow-dirty` packaged and compiled 0.1.1. The archive contains only `src`, `README.md`, and `LICENSE` plus Cargo metadata; its normalized manifest declares `license = "MIT"`. `Cargo.lock` changes only the shell-completions package version. The standard MIT text comes from [SPDX MIT license](https://spdx.org/licenses/MIT.txt), checked 2026-10-18.
