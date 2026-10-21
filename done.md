# crates — completed tasks

### T20.2. wasm-plugin-host: package layout

Plugin packages on disk, generic over the application's manifest type and home directory. `discover` finds user plugins under `<home>/plugins/<id>/versions/<digest12>/` (following `current`, with `link` winning for a development tree) and project plugins under `<project>/.<app>/plugins/<id>/`. A user plugin shadows a project plugin of the same id. `package_digest` is SHA-256 over the package tree. `layout` stages through a temp directory, swaps `current`/`previous`, writes `link`, and `remove`s only a path that canonicalizes inside `<home>/plugins/`. The manifest parse is the application's [`Manifest`]; this crate checks the id and refuses a wasm path that leaves the package. Tree copy, directory fsync and confined delete stay local until roadmap T16 `dir-ops`.

### T20.1. wasm-plugin-host: engine and worker

First slice of the shared WASM plugin host that cox and scull both build on (creator, 2026-10-07: extract the shared crate, `MIT OR Apache-2.0`). New crate `wasm-plugin-host`: `PluginHost` loads a module with extism (no WASI, no cache, a memory cap and a per-call deadline clamped to host maxima), runs it on a worker thread of its own with a control lane served before an event lane, cancels a call at its deadline through a watchdog, and maps extism failures to a typed `PluginError`. Host functions come from a `HostEnv` trait the application implements; the required export, thread names and limit defaults are options, so nothing names cox. Ported from cox `crates/cox-plugin/src/host.rs` and `error.rs` with no behaviour change. Done when the crate is in the workspace with the ported tests, registered for CI dry-run publish, bump, README and sonar.

Merged in pyrlyn/crates-packages#33 (`77ba2b1`): 10 tests, clippy, fmt, publish dry-run and `cargo +1.98 check` green. `rust.md` lists the crate (listepo/workspace#5, `0af0a5c`).

### T12. file-backup: subfolder target and keep-N pruning

rtok's agent SDK keeps its own copy of the folder mode (`BACKUP_DIR`, `backup`, `prune_backups`, `stale_backups` in `rtok-agent-sdk/src/lib.rs`) so it cannot depend on `file-backup`. Done means: `file-backup` offers that mode beside the existing sibling mode, with the same semantics and tests ported, so rtok can later swap its copy for the crate.

`Folder { name, keep }` with `backup`, `backup_at`, `prune` and `stale`, plus `DEFAULT_FOLDER = "_backup"`; `backup`/`backup_at` beside the file are unchanged. Both modes share one `create_new` copy loop and one byte-equality dedup check (symlinks and hard links to the source are not backups). A folder name that is not one plain path component is `InvalidInput`. Check: fmt, clippy `-D warnings` and `cargo test --workspace --locked` green (105 tests, 20 in `file-backup`).

### T11. Add the required project files

The project was missing `AGENTS.md`, `done.md`, `roadmap.md`, `ideas.md`, and `toolchain.md`. Done means: all files exist, and `toolchain.md` lists the workspace crates per the rulebook (also update `rust.md` then).

Those files are in the workspace root. `toolchain.md` lists `cargo-changed-packages`, `file-backup`, `git-changed-paths`, `path-gates`, and `scoped-check`. `rust.md` names the same five under shared crates. `file-backup` tests already live in `src/lib.rs`; the other four crates already have `tests/`.

### T1. Windows: POSIX quoting handed to cmd.exe in scoped-check

`rust/scoped-check/src/plan.rs:139,155-157` builds `{changed}`/`{packages}` with `shlex::try_join`/`try_quote` (POSIX single quotes), but `rust/scoped-check/src/main.rs:146-149` runs commands via `cmd /C` on Windows, which does not honor them. A file named `docs/x & calc & .md` executes `calc`; plain spaces break arguments. Tests are `#![cfg(unix)]` (`rust/scoped-check/tests/cli.rs:4`), so the path is untested. Done means: Windows-appropriate quoting (or raw argument passing) with tests covering it.

`src/quote.rs` quotes substituted values for the platform shell: `shlex` for `sh`, double quotes for `cmd.exe` (trailing backslashes doubled for the MSVC argument parser). `%`, `"` and control characters have no cmd.exe escape inside quotes, so such a value is a planning error (exit 3). Windows passes `cmd /D /V:OFF /S /C "<command>"` through `raw_arg`, because std's MSVC quoting of a single argument is not what cmd.exe parses; `/V:OFF` keeps `!` literal and `/D` skips AutoRun. Unit tests cover the cmd.exe quoter on every host; `tests/quoting.rs` runs a gate on paths with spaces, `&`, `'`, `^`, `;`, `!` and parentheses under the platform shell, plus a Windows-only `%` refusal test.

### T11. atomic-replace: one atomic file write for every project

Six projects (rtok, ketch, cox, swarfr, aulo, runa) carry about 18 hand-written temp-file-and-rename writes that disagree on fsync, permissions, symlinks and temp naming. New crate `atomic-replace` (std only, edition 2021, Rust 1.86 so ketch can use it): `write(path, bytes)` and `Options { durable, preserve_permissions, follow_symlinks, create_parent, mode }` with `write`/`write_with`, plus `sync_dir`. Ported from rtok `rtok-agent-sdk` `write_atomic`/`link_target` (permissions, Windows read-only bit, dangling links) and ketch `state.rs` `save_path` (fsync file and directory). Done means: the crate is in the workspace with tests, wired into CI dry-run publish, bump options, README and sonar; consumers migrate in their own tasks.

Plan:
1. `atomic-replace/` with Cargo.toml, src/lib.rs, AGENTS.md, README.md, plan/todo/done/roadmap/ideas/toolchain.md, justfile (copy of change-preview's layout).
2. Temp file beside the target via `create_new`, named `.<name>.tmp-<pid>-<nanos>-<counter>`; removed on every error path.
3. Register: workspace members, `ci.yml` dry-run, `bump.yml` package options, root README crate table and token scope, `sonar-project.properties`.
4. Check: `cargo test -p atomic-replace`, clippy `-D warnings`, fmt, `cargo +1.86 check -p atomic-replace`.

Delivered as the `write` module of `fs-kit` instead of a crate of its own (creator's call, 2026-10-12): `fs_kit::write::{atomic, Options, write_with, if_changed, sync_dir}`, ported from branch `t11-atomic-file` with its tests. Consumers migrate in their own tasks.

### T26. Classify Git output and external diff flags by their effects

`shell-classify` rated `git diff`, `git log`, and `git show` as read-only even when `--output` wrote a file or `--ext-diff` could execute a configured helper. Those flags now raise the risk to `Exec`, while ordinary reads remain `ReadOnly`. A regression test failed before the fix and passed after it; crate tests, clippy and formatting checks passed.

### T27. Preserve trailing newlines in coloured previews

`change_preview::paint` removed the final newline from a rendered diff, so subsequent terminal output could join its last line. It now preserves trailing newlines. A regression test failed before the fix and passed after it; crate tests, clippy and formatting checks passed.

### T28. Reuse parsed line diff in change-preview

`Preview::edit` parsed each before/after pair twice, once for counts and once for the unified diff. It now reuses one `TextDiff` through a private renderer. The public API and output stay the same; the existing tests, clippy and formatting checks passed before and after the refactor.

### T29. Refresh injecta's compile-fail diagnostic for Rust 1.99

The `single_thread_is_not_sync` trybuild diagnostic depended on whether standard-library source was available: macOS printed extra `Scope::spawn` context that Linux CI omitted. The fixture now requires `Sync` through a local function, preserving the rejection of `SingleThread` containers with a stable diagnostic. The focused compile-fail test passes; the full workspace tests, clippy and formatting passed before this fixture adjustment.


### T20.4. wasm-plugin-host: synchronize patched Extism and Wasmtime

Updated the shared WASM host and cox to Extism 1.41.0 / Wasmtime 49, removing the four now-fixed Wasmtime advisory ignores. The host API remains unchanged. Verified focused and workspace tests, clippy, fmt, and CI on pyrlyn/crates#74; no crate was manually published.

### T20.6. wasm-plugin-host and plugwright-extism: Pulley interpreter feature

Kerf's mobile plugin backend (kerf T15): iOS forbids executable memory at run time, so a host there cannot run wasmtime's native code. Feature `pulley` on both crates passes extism a `wasmtime::Config` whose target is Pulley (`pulley64`, or `pulley32` on 32-bit), wasmtime's own interpreter; extism keeps the target. The host API is unchanged. Done when both crates' full test suites pass with the feature on, a test checks that the engine built from the config is Pulley, and CI runs them.

### T20.5. abi-drift: permit Extism-compatible cbindgen patch

Lowered abi-drift's cbindgen minimum to 0.29.3, which Extism 1.41.0 pins exactly. Confirmed the API is compatible and generated C/C# outputs are unchanged. Verified abi-drift tests, workspace tests, clippy, fmt, and CI on pyrlyn/crates#74.

### T33. Reconcile release PR #39 with current workflow behavior

Resolved the older release branch against the current Rust package layout, preserving existing prerelease/dev-release options and feature-specific CI checks. Release helpers validate package names, publishability, versions and dependency release order; changelog notes come from the selected crate. Both historical file-backup changelog sections survive the move. Shared CI references use `@main` as in PR #90.

Six stubbed release-helper tests, shellcheck, workflow lint and diff checks pass locally. The tests run in CI; no release workflow was dispatched and nothing was published.

### T32. Correct speech preprocessing boundaries and reduce VAD allocations

The VAD compared already-padded spans, merging 300–600 ms pauses despite its documented 300 ms cutoff. It now merges unpadded frame spans only for gaps shorter than 300 ms, then adds the existing 150 ms context. Resampling rejects zero source or destination rates before empty-input shortcuts. Regression tests for 270/300/450/630 ms gaps and zero rates failed before the fixes and pass afterward; NaN-frame fallback and gap behavior are also pinned.

The VAD reuses its RMS buffer for speech decisions instead of allocating another per-frame mask, and uses slice chunks rather than manual frame slicing. Both resampling APIs share the existing rate validation through one private helper. The workspace search found no other VAD or rate-validation helper to reuse; no dependencies or public signatures changed.

Validation under mise: `cargo test -p speech-capture --locked` (28 tests), the same with `--no-default-features` (22 tests), `cargo nextest run -p speech-capture --locked` (28 tests; installed nextest 0.9.148), clippy with `--all-targets --locked -- -D warnings` for both feature sets, and `cargo fmt --all -- --check`. The optional whisper backend and full workspace were not rebuilt; no microphone or model is needed for the new tests.

### T30. Extract reusable shell completion generation

Use the local `shell-completions` crate in `packages/crates/rust`. Preserve rtok scripts and ketch package-name completion and installation behavior. Add `cox completions <shell>` without loading configuration, credentials, telemetry or a session.

Execution plan: establish existing completion tests; extract the clap generator and Clink implementation with regression tests into the shared crate; connect consumers through local Cargo paths; update dependency inventories; run focused tests, formatting and clippy. Application-specific installation policies and ketch dynamic package lookup stay with their owners. Done when generation has one owner and the three CLI surfaces pass their checks.

Validation: Four generator tests passed, including upstream byte parity for five shells, Clink aliases/hidden values/escaping and writer errors. Formatting and all-target clippy passed. Consumer checks verified all 11 existing application scripts byte for byte. Existing dependency versions and unrelated lockfile edges were preserved.

Reused `clap_complete` and the existing rtok Clink generator. No replacement library or custom generator was introduced. Local path dependencies resolve in the standard apps/packages layout.

### T34. Integrate remaining local build settings

Ported the three remaining local build-setting commits onto current main: optional sccache, Linux mold through mise, and shared CI R2 cache support. Existing tool pins and CI behavior remain intact; only the required R2 credentials are forwarded. Updated the toolchain inventory and existing Windows/Cranelift documentation. Reused the existing shared workflow rather than adding cache logic.

Validation: mise renders the macOS wrapper and empty Linux flags correctly; the recursion guard clears the wrapper. All 20 existing file-backup tests, all-target clippy, workspace formatting and jactionlint pass. Real compilation reaches sccache. This configuration-only integration uses existing tests and the required platform CI instead of a new test harness.

### T35. Fix publish workflow bootstrap after enabling sccache

Added the existing pinned sccache tool to publish-crates mise installation. The cargo backend requires its configured compiler wrapper even when installing cargo-workspaces; the restricted tool list omitted it and failed before any publish command (main run 39146935615). Publication gates and pinned versions are unchanged.

Workflow lint and diff checks pass locally. The existing pull-request publish job exercises a clean Linux bootstrap and dry-run publication as integration regression proof; it uploads no crates on pull requests.

### T37. Move plugwright under rust/ and onto the release path

plugwright lived in its own workspace at `plugwright/`, which bump.yml, release.yml and release-crate.sh never see, so none of its crates could reach crates.io and rtok's CI could not depend on it. Moved it to `rust/plugwright/` and made its 18 publishable crates and 4 examples members of the root workspace: one `Cargo.lock`, the root toolchain (1.99.0), the root `[workspace.dependencies]`. Each crate's metadata is explicit, because inheriting the root's GPL license would have changed plugwright's MIT OR Apache-2.0. Each crate has a release-plz `CHANGELOG.md`, and the README lists the release order. `plugwright-wasmer` stays outside the workspace (wasmer 7.5 and wasmtime 49 pin incompatible cranelift versions) and is `publish = false`. The separate `plugwright` CI job folded into the root `rust` job on all four targets; the wasm32 target is added there and in release.yml's gate.

Validation: workspace fmt, all-target clippy -D warnings and `cargo test --workspace --locked` pass on 1.99.0, as do the loader's native-only clippy, the SDK's wasm32 clippy, the bench smoke run and publish dry-runs for the first release step. The one test that failed on 1.99 asserted std's `TryFromIntError` wording, which changed in that release; it now compares against the error's own `Display`, which fails on 1.99 before the change and passes after it. The 19 crate names returned 404 from the crates.io API on 2026-10-19. Nothing is published here: the first release is a `Bump and release` run per crate.

### T31. Publish shell-completions through the managed release pipeline

Publish the extracted crate so independently checked-out applications build in CI. Execution plan: enable publication in the crate manifest, add it to the existing bump workflow and publish dry-run gate, verify packaging and tests, merge the shared-crate PR, dispatch Bump and release for shell-completions, and verify the version on crates.io before consumers adopt it. The release-untagged-head version step reads the selected package version without planning unrelated workspace bumps; test both existing and unknown packages. No manual cargo publish. Done when the managed pipeline publishes the crate and downstream registry dependencies resolve.

Validation: `cargo publish --dry-run -p shell-completions --locked` and all four generator tests passed. The release-current regression covers selected and unknown packages. PR #92 was merged; managed bump and release published version 0.1.0 through tag `shell-completions-v0.1.0`. The initial tag attempt failed after main moved during its checks; a new managed run on current main completed without permission changes or manual publication.

Publication: https://crates.io/crates/shell-completions/0.1.0 and https://github.com/pyrlyn/crates/actions/runs/39146869914. All three downstream lockfiles resolve the registry package with checksum `4608c4bf4c4178d17877ff5c2c5631c5d414c64f1a76346adbdfab8fd7c6bf12`; standalone locked metadata passed in isolated Git archives. Existing dependency versions were preserved.
