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

### T20.5. abi-drift: permit Extism-compatible cbindgen patch

Lowered abi-drift's cbindgen minimum to 0.29.3, which Extism 1.41.0 pins exactly. Confirmed the API is compatible and generated C/C# outputs are unchanged. Verified abi-drift tests, workspace tests, clippy, fmt, and CI on pyrlyn/crates#74.

### T30. Correct speech preprocessing boundaries and reduce VAD allocations

The VAD compared already-padded spans, merging 300–600 ms pauses despite its documented 300 ms cutoff. It now merges unpadded frame spans only for gaps shorter than 300 ms, then adds the existing 150 ms context. Resampling rejects zero source or destination rates before empty-input shortcuts. Regression tests for 270/300/450/630 ms gaps and zero rates failed before the fixes and pass afterward; NaN-frame fallback and gap behavior are also pinned.

The VAD reuses its RMS buffer for speech decisions instead of allocating another per-frame mask, and uses slice chunks rather than manual frame slicing. Both resampling APIs share the existing rate validation through one private helper. The workspace search found no other VAD or rate-validation helper to reuse; no dependencies or public signatures changed.

Validation under mise: `cargo test -p speech-capture --locked` (28 tests), the same with `--no-default-features` (22 tests), `cargo nextest run -p speech-capture --locked` (28 tests; installed nextest 0.9.148), clippy with `--all-targets --locked -- -D warnings` for both feature sets, and `cargo fmt --all -- --check`. The optional whisper backend and full workspace were not rebuilt; no microphone or model is needed for the new tests.
